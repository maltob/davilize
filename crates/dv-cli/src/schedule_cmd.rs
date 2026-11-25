//! `davilize schedule`: list a report's schedules, and run them.
//!
//! `schedule run --now` runs the selected schedules once and exits, for Windows Task
//! Scheduler or cron. Without `--now` the command is a foreground daemon: it sleeps
//! until the next due schedule, runs it, and repeats. Each run reloads the report, so
//! edits apply from the next run, and a failed run never stops the daemon.
//!
//! The decision logic (next due time, which schedules are due, which are selected) is
//! in pure functions below and is unit tested without sleeping.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, SecondsFormat};
use clap::ValueEnum;
use dv_core::{CoreError, Report, Schedule, ScheduleAction, describe_cron};
use serde_json::{Value, json};

use crate::pipeline;
use crate::publish_cmd::{self, PublishFormat, PublishRequest};
use crate::{CliError, CliResult, DataOutput, EXIT_INPUT, Log, base_dir, output_error};

/// How long the daemon waits before trying again after a failed report load.
const RETRY: Duration = Duration::from_secs(60);
/// The longest a sleep goes without checking for Ctrl+C.
const SLICE: Duration = Duration::from_secs(1);

/// Settings shared by every run of one `schedule run` invocation.
pub struct RunOptions {
    /// Vendored vega bundles, used only with `--offline`.
    pub vendor_dir: Option<PathBuf>,
    pub trust_new_host_key: bool,
}

/// A schedule the daemon may run, with the start time of its last run, if any.
pub struct Candidate<'a> {
    pub schedule: &'a Schedule,
    pub last_run: Option<DateTime<Local>>,
}

/// The next time a candidate is due: its first run after `now` or after its last run,
/// whichever is later. Slots missed while the daemon was busy are skipped, not replayed,
/// and a slot is never run twice.
pub fn next_due(
    candidate: &Candidate,
    now: DateTime<Local>,
) -> Result<Option<DateTime<Local>>, CoreError> {
    let anchor = match candidate.last_run {
        Some(last) => last.max(now),
        None => now,
    };
    Ok(candidate.schedule.next_runs(anchor, 1)?.into_iter().next())
}

/// The names whose due time has come by `now`, in the order given.
pub fn due_names(plan: &[(String, Option<DateTime<Local>>)], now: DateTime<Local>) -> Vec<String> {
    plan.iter()
        .filter(|(_, at)| at.is_some_and(|at| at <= now))
        .map(|(name, _)| name.clone())
        .collect()
}

/// The schedules a command acts on. Without names: the enabled ones. With names: the
/// named ones, which `--now` runs even when disabled (the user asked for them by name);
/// the daemon only runs named schedules that are enabled. An unknown name is an input error.
fn select<'a>(report: &'a Report, names: &[String], daemon: bool) -> CliResult<Vec<&'a Schedule>> {
    for name in names {
        if find(report, name).is_none() {
            let declared: Vec<&str> = report.schedules.iter().map(|s| s.name.as_str()).collect();
            let known = if declared.is_empty() {
                "none".to_string()
            } else {
                declared.join(", ")
            };
            return Err(CliError {
                code: EXIT_INPUT,
                message: format!("unknown schedule '{name}' (declared: {known})"),
            });
        }
    }
    Ok(report
        .schedules
        .iter()
        .filter(|schedule| {
            if names.is_empty() {
                schedule.enabled
            } else {
                names.iter().any(|n| n.eq_ignore_ascii_case(&schedule.name))
                    && (!daemon || schedule.enabled)
            }
        })
        .collect())
}

fn find<'a>(report: &'a Report, name: &str) -> Option<&'a Schedule> {
    report
        .schedules
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
}

/// `davilize schedule list`: name, state, cron and its description, action, formats,
/// and the next run times (local time, RFC 3339).
pub fn list(report_path: &Path, count: usize, log: &Log) -> CliResult<()> {
    let report = Report::load(report_path)?;
    let now = Local::now();
    let mut upcoming = Vec::new();
    for schedule in &report.schedules {
        let next = if schedule.enabled {
            schedule.next_runs(now, count)?
        } else {
            Vec::new()
        };
        upcoming.push(next);
    }

    if log.json {
        let items: Vec<Value> = report
            .schedules
            .iter()
            .zip(&upcoming)
            .map(|(schedule, next)| {
                json!({
                    "name": schedule.name,
                    "enabled": schedule.enabled,
                    "cron": schedule.cron,
                    "description": describe_cron(&schedule.cron),
                    "action": schedule.action,
                    "formats": schedule.effective_formats(),
                    "params": schedule.params,
                    "next_runs": next.iter().map(rfc3339).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Array(items)).map_err(output_error)?
        );
    } else if report.schedules.is_empty() {
        println!("(no schedules)");
    } else {
        for (schedule, next) in report.schedules.iter().zip(&upcoming) {
            print_schedule(schedule, next);
        }
    }
    log.info(
        &format!("{} schedules in '{}'", report.schedules.len(), report.name),
        json!({"report": report.name, "schedules": report.schedules.len()}),
    );
    Ok(())
}

fn print_schedule(schedule: &Schedule, next: &[DateTime<Local>]) {
    let state = if schedule.enabled {
        "enabled"
    } else {
        "disabled"
    };
    println!("{}  {state}", schedule.name);
    println!(
        "  cron:    {} ({})",
        schedule.cron,
        describe_cron(&schedule.cron)
    );
    println!("  action:  {}", action_text(&schedule.action));
    println!("  formats: {}", schedule.effective_formats().join(", "));
    if !schedule.params.is_empty() {
        let pairs: Vec<String> = schedule
            .params
            .iter()
            .map(|p| format!("{}={}", p.name, p.value))
            .collect();
        println!("  params:  {}", pairs.join(" "));
    }
    if !schedule.enabled {
        println!("  next:    (disabled)");
    } else if next.is_empty() {
        println!("  next:    none");
    } else {
        println!("  next:");
        for time in next {
            println!("    {}", rfc3339(time));
        }
    }
}

fn action_text(action: &ScheduleAction) -> String {
    match action {
        ScheduleAction::Refresh { out } => format!("refresh to {}", out.display()),
        ScheduleAction::Publish { target } => format!("publish to {target}"),
    }
}

fn rfc3339(time: &DateTime<Local>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// `davilize schedule run`: once with `--now`, otherwise as the daemon.
pub fn run(
    report_path: &Path,
    names: &[String],
    now_only: bool,
    options: &RunOptions,
    log: &Log,
) -> CliResult<()> {
    if now_only {
        run_once(report_path, names, options, log)
    } else {
        daemon(report_path, names, options, log)
    }
}

/// Runs the selected schedules once, in order. A failure does not stop the others.
/// The exit status is that of the first failure.
fn run_once(
    report_path: &Path,
    names: &[String],
    options: &RunOptions,
    log: &Log,
) -> CliResult<()> {
    let report = Report::load(report_path)?;
    let chosen: Vec<String> = select(&report, names, false)?
        .iter()
        .map(|s| s.name.clone())
        .collect();
    if chosen.is_empty() {
        log.warn(
            "no enabled schedules to run",
            json!({"report": report.name, "status": "nothing_to_run"}),
        );
        return Ok(());
    }

    let mut failures = 0;
    let mut first: Option<CliError> = None;
    for name in &chosen {
        if let Err(err) = run_named(report_path, name, false, options, log) {
            failures += 1;
            first.get_or_insert(err);
        }
    }
    match first {
        None => Ok(()),
        Some(err) => Err(CliError {
            code: err.code,
            message: format!(
                "{failures} of {} schedules failed; the first: {}",
                chosen.len(),
                err.message
            ),
        }),
    }
}

/// The foreground daemon: wait for the earliest due schedule, run whatever is due,
/// repeat. Ctrl+C stops it after the current run.
fn daemon(report_path: &Path, names: &[String], options: &RunOptions, log: &Log) -> CliResult<()> {
    // A bad report or an unknown name stops the daemon at start, before it begins.
    let report = Report::load(report_path)?;
    select(&report, names, true)?;

    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst))
        .map_err(|e| output_error(format!("cannot install the Ctrl+C handler: {e}")))?;
    log.info(
        &format!("schedule daemon started for '{}'", report.name),
        json!({"report": report.name, "status": "started"}),
    );

    let mut last_run: HashMap<String, DateTime<Local>> = HashMap::new();
    while !stop.load(Ordering::SeqCst) {
        // Reloaded every round, so added, removed and edited schedules take effect.
        let report = match Report::load(report_path) {
            Ok(report) => report,
            Err(err) => {
                log.error_with(
                    &format!(
                        "cannot load the report, retrying in {} s: {err}",
                        RETRY.as_secs()
                    ),
                    json!({"status": "retry"}),
                );
                if !pause(&stop, RETRY) {
                    break;
                }
                continue;
            }
        };
        let selected = match select(&report, names, true) {
            Ok(selected) => selected,
            Err(err) => {
                log.error_with(&err.message, json!({"status": "retry"}));
                if !pause(&stop, RETRY) {
                    break;
                }
                continue;
            }
        };

        let now = Local::now();
        let mut plan = Vec::new();
        for schedule in selected {
            let candidate = Candidate {
                schedule,
                last_run: last_run.get(&schedule.name.to_ascii_lowercase()).copied(),
            };
            match next_due(&candidate, now) {
                Ok(at) => plan.push((schedule.name.clone(), at)),
                Err(err) => {
                    log.error_with(
                        &format!("schedule '{}' skipped: {err}", schedule.name),
                        json!({"schedule": schedule.name, "status": "skipped"}),
                    );
                }
            }
        }

        let Some(next) = plan.iter().filter_map(|(_, at)| *at).min() else {
            // Nothing enabled: look again later, so a schedule added to the report is picked up.
            if !pause(&stop, RETRY) {
                break;
            }
            continue;
        };
        if !wait_until(&stop, next) {
            break;
        }

        for name in due_names(&plan, Local::now()) {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            last_run.insert(name.to_ascii_lowercase(), Local::now());
            // Failures are logged inside run_named; the daemon keeps going.
            let _ = run_named(report_path, &name, true, options, log);
        }
    }
    log.info("schedule daemon stopped", json!({"status": "stopped"}));
    Ok(())
}

/// Runs one schedule. The report is reloaded first, so the run uses the current
/// definition. Logs a start line, then an ok or failed line with the duration.
/// With `daemon`, a schedule that has been disabled since it was planned is skipped.
fn run_named(
    report_path: &Path,
    name: &str,
    daemon: bool,
    options: &RunOptions,
    log: &Log,
) -> CliResult<()> {
    log.info(
        &format!("schedule '{name}' started"),
        json!({"schedule": name, "status": "started"}),
    );
    let started = Instant::now();
    let outcome = Report::load(report_path)
        .map_err(CliError::from)
        .and_then(|report| {
            let Some(schedule) = find(&report, name) else {
                return Err(CliError {
                    code: EXIT_INPUT,
                    message: format!("schedule '{name}' is no longer in the report"),
                });
            };
            if daemon && !schedule.enabled {
                return Ok(());
            }
            execute(report_path, schedule, options, log)
        });

    let elapsed = started.elapsed();
    let seconds = format!("{:.1}s", elapsed.as_secs_f64());
    let duration_ms = elapsed.as_millis() as u64;
    match &outcome {
        Ok(()) => log.info(
            &format!("schedule '{name}' ok in {seconds}"),
            json!({"schedule": name, "status": "ok", "duration_ms": duration_ms}),
        ),
        Err(err) => log.error_with(
            &format!("schedule '{name}' failed in {seconds}: {}", err.message),
            json!({"schedule": name, "status": "failed", "duration_ms": duration_ms}),
        ),
    }
    outcome
}

/// Does the work of one schedule through the same paths as `refresh` and `publish`.
fn execute(
    report_path: &Path,
    schedule: &Schedule,
    options: &RunOptions,
    log: &Log,
) -> CliResult<()> {
    let params: Vec<(String, String)> = schedule
        .params
        .iter()
        .map(|p| (p.name.clone(), p.value.clone()))
        .collect();
    let format_names = schedule.effective_formats();
    match &schedule.action {
        ScheduleAction::Refresh { out } => {
            let formats = format_names
                .iter()
                .map(|name| parse_format::<DataOutput>(name))
                .collect::<CliResult<Vec<_>>>()?;
            let out_dir = resolve_local(report_path, out);
            let pipeline_options = pipeline::Options {
                formats,
                vendor_dir: options.vendor_dir.clone(),
                html_name: None,
                params,
            };
            pipeline::run(report_path, &out_dir, &pipeline_options, log)?;
            Ok(())
        }
        ScheduleAction::Publish { target } => {
            let formats = format_names
                .iter()
                .map(|name| parse_format::<PublishFormat>(name))
                .collect::<CliResult<Vec<_>>>()?;
            let target = resolve_target(report_path, target);
            publish_cmd::publish_report(
                PublishRequest {
                    report: report_path,
                    target: &target,
                    formats: &formats,
                    vendor_dir: options.vendor_dir.as_deref(),
                    dry_run: false,
                    trust_new_host_key: options.trust_new_host_key,
                    params,
                },
                log,
            )
        }
    }
}

/// A format name from the report, through the CLI's own `--format` names. The core
/// has already checked the names, so this only fails if the two lists drift apart.
fn parse_format<T: ValueEnum>(name: &str) -> CliResult<T> {
    T::from_str(name, true).map_err(|reason| CliError {
        code: EXIT_INPUT,
        message: format!("format '{name}' is not supported: {reason}"),
    })
}

/// A relative output folder is resolved against the report's folder.
fn resolve_local(report_path: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir(report_path).join(path)
    }
}

/// A relative local publish target is resolved against the report's folder. URLs
/// (`file://`, `s3://`, `sftp://`) are used as they are.
fn resolve_target(report_path: &Path, target: &str) -> String {
    if target.contains("://") || Path::new(target).is_absolute() {
        target.to_string()
    } else {
        base_dir(report_path).join(target).display().to_string()
    }
}

/// Sleeps for `total` in short slices. Returns false if Ctrl+C was pressed first.
fn pause(stop: &AtomicBool, total: Duration) -> bool {
    let deadline = Instant::now() + total;
    loop {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(SLICE));
    }
}

/// Sleeps until local time `at`. Returns false if Ctrl+C was pressed first.
fn wait_until(stop: &AtomicBool, at: DateTime<Local>) -> bool {
    let remaining = (at - Local::now()).to_std().unwrap_or(Duration::ZERO);
    pause(stop, remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn local(h: u32, mi: u32, s: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2024, 1, 1, h, mi, s)
            .single()
            .expect("unambiguous local time")
    }

    fn schedule(name: &str, cron: &str, enabled: bool) -> Schedule {
        Schedule {
            name: name.into(),
            cron: cron.into(),
            enabled,
            formats: vec![],
            params: vec![],
            action: ScheduleAction::Refresh { out: "out".into() },
        }
    }

    fn report_with(schedules: Vec<Schedule>) -> Report {
        let mut report = Report::new("r");
        report.schedules = schedules;
        report
    }

    #[test]
    fn next_due_is_the_first_slot_after_now() {
        let s = schedule("a", "*/15 * * * *", true);
        let candidate = Candidate {
            schedule: &s,
            last_run: None,
        };
        assert_eq!(
            next_due(&candidate, local(10, 7, 0)).unwrap(),
            Some(local(10, 15, 0))
        );
    }

    #[test]
    fn a_slot_is_never_run_twice() {
        let s = schedule("a", "*/15 * * * *", true);
        // Started at 10:15:00 and it is 10:15:00.2 now: the next run is 10:30.
        let candidate = Candidate {
            schedule: &s,
            last_run: Some(local(10, 15, 0)),
        };
        let now = local(10, 15, 0) + chrono::TimeDelta::milliseconds(200);
        assert_eq!(next_due(&candidate, now).unwrap(), Some(local(10, 30, 0)));
    }

    #[test]
    fn missed_slots_are_skipped_not_replayed() {
        let s = schedule("a", "*/15 * * * *", true);
        // Last started 10:15, but the daemon only looks again at 11:50: 10:30 to 11:45
        // are gone and the next run is 12:00, once.
        let candidate = Candidate {
            schedule: &s,
            last_run: Some(local(10, 15, 0)),
        };
        assert_eq!(
            next_due(&candidate, local(11, 50, 0)).unwrap(),
            Some(local(12, 0, 0))
        );
    }

    #[test]
    fn due_names_selects_only_schedules_whose_time_has_come() {
        let plan = vec![
            ("early".to_string(), Some(local(10, 15, 0))),
            ("late".to_string(), Some(local(10, 30, 0))),
            ("none".to_string(), None),
        ];
        assert_eq!(due_names(&plan, local(10, 15, 30)), vec!["early"]);
        assert_eq!(due_names(&plan, local(10, 30, 0)), vec!["early", "late"]);
        assert!(due_names(&plan, local(10, 0, 0)).is_empty());
    }

    #[test]
    fn select_by_default_takes_enabled_schedules_only() {
        let report = report_with(vec![
            schedule("on", "0 7 * * *", true),
            schedule("off", "0 8 * * *", false),
        ]);
        let names: Vec<&str> = select(&report, &[], false)
            .unwrap()
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, vec!["on"]);
    }

    #[test]
    fn named_disabled_schedules_run_only_with_now() {
        let report = report_with(vec![schedule("off", "0 8 * * *", false)]);
        let named = vec!["OFF".to_string()];
        assert_eq!(select(&report, &named, false).unwrap().len(), 1);
        assert!(select(&report, &named, true).unwrap().is_empty());
    }

    #[test]
    fn unknown_names_are_input_errors() {
        let report = report_with(vec![schedule("on", "0 7 * * *", true)]);
        let err = select(&report, &["nope".to_string()], true).unwrap_err();
        assert_eq!(err.code, EXIT_INPUT);
        assert!(err.message.contains("unknown schedule 'nope'"));
        assert!(err.message.contains("declared: on"));
    }

    #[test]
    fn resolve_paths_against_the_report_folder() {
        let report = Path::new("/work/reports/sales.dvz");
        assert_eq!(
            resolve_local(report, Path::new("out")),
            Path::new("/work/reports").join("out")
        );
        assert_eq!(
            resolve_target(report, "s3://bucket/prefix"),
            "s3://bucket/prefix"
        );
        assert_eq!(
            resolve_target(report, "sftp://me@host/x"),
            "sftp://me@host/x"
        );
    }
}
