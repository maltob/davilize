//! Smoke tests for `davilize schedule`: the listing, `run --now` for a named schedule,
//! unknown names, and exit codes when a schedule fails. The example report is copied into
//! a temporary folder first, because its schedule writes to a relative `out` folder next
//! to the report; nothing is written into the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|_| panic!("examples/{name} exists"))
}

/// Copies the scheduled sample and its data file into `dir`; returns the copied report.
fn copy_report(dir: &Path) -> PathBuf {
    std::fs::copy(example("sample.csv"), dir.join("sample.csv")).unwrap();
    let report = dir.join("sample_scheduled.dvz");
    std::fs::copy(example("sample_scheduled.dvz"), &report).unwrap();
    report
}

fn davilize(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args(args)
        .output()
        .expect("davilize runs")
}

#[test]
fn list_shows_the_cron_reading_action_and_next_runs() {
    let report = example("sample_scheduled.dvz");
    let out = davilize(&["schedule", "list", report.to_str().unwrap(), "--count", "2"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("weekday_refresh  enabled"), "{text}");
    assert!(text.contains("30 7 * * 1-5 (weekdays at 07:30)"), "{text}");
    assert!(text.contains("refresh to out"), "{text}");
    assert!(text.contains("formats: csv"), "{text}");
    let next: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("    20"))
        .collect();
    assert_eq!(next.len(), 2, "{text}");
    assert!(next.iter().all(|line| line.contains("T07:30:00")), "{text}");
}

#[test]
fn list_json_is_an_array_with_the_same_facts() {
    let report = example("sample_scheduled.dvz");
    let out = davilize(&[
        "--json",
        "schedule",
        "list",
        report.to_str().unwrap(),
        "--count",
        "1",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let first = &value[0];
    assert_eq!(first["name"], "weekday_refresh");
    assert_eq!(first["enabled"], true);
    assert_eq!(first["description"], "weekdays at 07:30");
    assert_eq!(first["action"]["type"], "refresh");
    assert_eq!(first["next_runs"].as_array().unwrap().len(), 1);
}

#[test]
fn run_now_by_name_writes_the_refresh_outputs_next_to_the_report() {
    let dir = tempfile::tempdir().unwrap();
    let report = copy_report(dir.path());
    let out = davilize(&[
        "schedule",
        "run",
        report.to_str().unwrap(),
        "--now",
        "--name",
        "WEEKDAY_REFRESH",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        dir.path()
            .join("out")
            .join("revenue_by_region.csv")
            .exists()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("schedule 'weekday_refresh' ok"), "{stderr}");
}

#[test]
fn unknown_schedule_names_are_input_errors() {
    let dir = tempfile::tempdir().unwrap();
    let report = copy_report(dir.path());
    let now = davilize(&[
        "schedule",
        "run",
        report.to_str().unwrap(),
        "--now",
        "--name",
        "nope",
    ]);
    assert_eq!(now.status.code(), Some(3));

    // The daemon checks names at startup too, so it fails at once instead of waiting.
    let daemon = davilize(&[
        "schedule",
        "run",
        report.to_str().unwrap(),
        "--name",
        "nope",
    ]);
    assert_eq!(daemon.status.code(), Some(3));
    assert!(!dir.path().join("out").exists());
}

#[test]
fn a_failing_schedule_sets_the_exit_code_and_the_others_still_run() {
    let dir = tempfile::tempdir().unwrap();
    let report = copy_report(dir.path());

    // Turn the schedule into two: one whose folder cannot be created (a file is in the
    // way) and one that works. The first failure's code is 5 (output problem).
    std::fs::write(dir.path().join("blocker"), "not a folder").unwrap();
    let text = std::fs::read_to_string(&report).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&text).unwrap();
    value["schedules"] = serde_json::json!([
        {
            "name": "bad",
            "cron": "0 6 * * *",
            "action": {"type": "refresh", "out": "blocker/inside"}
        },
        {
            "name": "good",
            "cron": "0 6 * * *",
            "action": {"type": "refresh", "out": "good"}
        }
    ]);
    std::fs::write(&report, serde_json::to_string_pretty(&value).unwrap()).unwrap();

    let out = davilize(&["schedule", "run", report.to_str().unwrap(), "--now"]);
    assert_eq!(
        out.status.code(),
        Some(5),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("schedule 'bad' failed"), "{stderr}");
    assert!(stderr.contains("schedule 'good' ok"), "{stderr}");
    assert!(
        dir.path()
            .join("good")
            .join("revenue_by_region.csv")
            .exists()
    );
}
