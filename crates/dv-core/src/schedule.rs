//! Report schedules: when a report refreshes or publishes, and the cron rules that
//! decide when.
//!
//! A schedule is a standard 5-field cron expression in local time:
//! `minute hour day-of-month month day-of-week`. Day of week is 0 to 6 with 0 and 7
//! both meaning Sunday; month and day names (`jan`, `mon`) are accepted. As in
//! vixie cron, when both the day-of-month and the day-of-week are restricted (neither
//! field starts with `*`), a run happens when either one matches.
//!
//! Pure: no files and no clock. Callers pass in the instant to start from.
//! See docs/reports.md ("Schedules").

use std::collections::{BTreeSet, HashSet};
use std::path::PathBuf;
use std::str::FromStr;

use chrono::{DateTime, Local};
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};

use crate::CoreError;
use crate::model::{Parameter, check_name, invalid};
use crate::params::resolve_parameters;

/// Formats a refresh schedule may write: the names `davilize refresh --format` takes.
pub const SCHEDULE_FORMATS: &[&str] = &["html", "csv", "parquet", "svg", "png", "pdf", "xlsx"];

/// Formats a publish schedule may include: the names `davilize publish --format` takes.
/// SVG is not published (it is per-visual and large), so it is refresh-only.
pub const PUBLISH_SCHEDULE_FORMATS: &[&str] = &["html", "csv", "parquet", "png", "pdf", "xlsx"];

const CUSTOM: &str = "custom schedule";
const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const WEEKDAY_TITLES: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// A scheduled refresh or publish of the report. See docs/reports.md ("Schedules").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schedule {
    /// Unique within the report, compared case-insensitively. Same rules as visual ids.
    pub name: String,
    /// Five fields, `minute hour day-of-month month day-of-week`, in local time.
    pub cron: String,
    /// A disabled schedule is kept but never run. Omitted from the JSON when enabled.
    #[serde(default = "default_enabled", skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    /// Output formats, named as `--format` names them. Empty means `html` for a
    /// publish and `csv` for a refresh.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<String>,
    /// Parameter values for each run. Names must be declared parameters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ScheduleParam>,
    pub action: ScheduleAction,
}

fn default_enabled() -> bool {
    true
}

fn is_enabled(value: &bool) -> bool {
    *value
}

/// A parameter value for a scheduled run, as `NAME=VALUE` on the command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleParam {
    pub name: String,
    pub value: String,
}

/// What a schedule does when it is due.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScheduleAction {
    /// Write the outputs into a folder. A relative folder is resolved against the
    /// report's folder.
    Refresh { out: PathBuf },
    /// Refresh and upload to a target: a folder, `file://`, `s3://` or `sftp://`.
    /// Credentials never go in the target; they come from the environment.
    Publish { target: String },
}

impl Schedule {
    /// The formats a run writes: the listed ones, or the default for the action.
    pub fn effective_formats(&self) -> Vec<String> {
        if !self.formats.is_empty() {
            return self.formats.clone();
        }
        match self.action {
            ScheduleAction::Refresh { .. } => vec!["csv".to_string()],
            ScheduleAction::Publish { .. } => vec!["html".to_string()],
        }
    }

    /// The next `count` run times strictly after `after`, earliest first, in local time.
    /// Fewer are returned when the expression never matches again.
    pub fn next_runs(
        &self,
        after: DateTime<Local>,
        count: usize,
    ) -> Result<Vec<DateTime<Local>>, CoreError> {
        let schedules = cron_schedules(&self.cron)
            .map_err(|reason| invalid(format!("invalid cron '{}': {reason}", self.cron)))?;
        let mut runs: Vec<DateTime<Local>> = schedules
            .iter()
            .flat_map(|schedule| schedule.after(&after).take(count))
            .collect();
        runs.sort();
        runs.dedup();
        runs.truncate(count);
        Ok(runs)
    }
}

/// Checks the schedules of a report against its parameters. Called by `Report::validate`.
pub(crate) fn check_schedules(
    schedules: &[Schedule],
    parameters: &[Parameter],
) -> Result<(), CoreError> {
    let mut names = HashSet::new();
    for schedule in schedules {
        check_name("schedule", &schedule.name)?;
        if !names.insert(schedule.name.to_ascii_lowercase()) {
            return Err(invalid(format!(
                "duplicate schedule name '{}' (names are case-insensitive)",
                schedule.name
            )));
        }
        check_one(schedule, parameters)
            .map_err(|reason| invalid(format!("schedule '{}': {reason}", schedule.name)))?;
    }
    Ok(())
}

fn check_one(schedule: &Schedule, parameters: &[Parameter]) -> Result<(), String> {
    cron_schedules(&schedule.cron)
        .map_err(|reason| format!("invalid cron '{}': {reason}", schedule.cron))?;

    let allowed = match schedule.action {
        ScheduleAction::Refresh { .. } => SCHEDULE_FORMATS,
        ScheduleAction::Publish { .. } => PUBLISH_SCHEDULE_FORMATS,
    };
    let mut seen_formats = HashSet::new();
    for format in &schedule.formats {
        if !allowed.contains(&format.as_str()) {
            return Err(format!(
                "format '{format}' is not one of: {}",
                allowed.join(", ")
            ));
        }
        if !seen_formats.insert(format.as_str()) {
            return Err(format!("format '{format}' is listed twice"));
        }
    }

    let mut seen_params = HashSet::new();
    for param in &schedule.params {
        let declared = parameters
            .iter()
            .find(|p| p.name == param.name)
            .ok_or_else(|| format!("parameter '{}' is not declared in the report", param.name))?;
        if !seen_params.insert(param.name.as_str()) {
            return Err(format!("parameter '{}' is set twice", param.name));
        }
        // Checked with the parameter's own type and constraints. Only that one
        // parameter is passed, so required parameters without a value do not matter here.
        resolve_parameters(
            std::slice::from_ref(declared),
            &[(param.name.clone(), param.value.clone())],
        )
        .map_err(|err| match err {
            CoreError::Invalid(reason) => reason,
            other => other.to_string(),
        })?;
    }

    match &schedule.action {
        ScheduleAction::Refresh { out } => {
            if out.as_os_str().is_empty() {
                return Err("the refresh folder 'out' must not be empty".to_string());
            }
        }
        ScheduleAction::Publish { target } => check_publish_target(target)?,
    }
    Ok(())
}

/// A publish target must not carry a password. `user:pass@host` in the authority of a
/// URL is rejected; a user name alone (`sftp://user@host`) is fine. Credentials come
/// from the environment (docs/publishing.md).
pub fn check_publish_target(target: &str) -> Result<(), String> {
    if target.trim().is_empty() {
        return Err("the publish target must not be empty".to_string());
    }
    if let Some((_, rest)) = target.split_once("://") {
        let authority = rest.split('/').next().unwrap_or("");
        if let Some((userinfo, _)) = authority.rsplit_once('@')
            && userinfo.contains(':')
        {
            return Err(
                "the publish target must not contain a password; put credentials in the environment (see docs/publishing.md)"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// Parses a 5-field expression into the `cron` crate's schedules. Usually one; two
/// when both day fields are restricted, because a run then happens when either matches.
fn cron_schedules(expr: &str) -> Result<Vec<CronSchedule>, String> {
    let fields = parse_fields(expr)?;
    let restrict_days = !fields.days_star;
    let restrict_weekdays = !fields.weekdays_star;
    let forms = if restrict_days && restrict_weekdays {
        vec![
            crate_expression(&fields, true, false),
            crate_expression(&fields, false, true),
        ]
    } else {
        vec![crate_expression(&fields, restrict_days, restrict_weekdays)]
    };
    forms
        .iter()
        .map(|form| CronSchedule::from_str(form).map_err(|err| err.to_string()))
        .collect()
}

/// The `cron` crate's six-field form (seconds first) with explicit values. Its
/// day-of-week runs 1 (Sunday) to 7 (Saturday), so standard day numbers are shifted by one.
fn crate_expression(fields: &Fields, days: bool, weekdays: bool) -> String {
    let list = |set: &BTreeSet<u32>, shift: u32| {
        set.iter()
            .map(|value| (value + shift).to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let day_of_month = if days {
        list(&fields.days, 0)
    } else {
        "*".to_string()
    };
    let day_of_week = if weekdays {
        list(&fields.weekdays, 1)
    } else {
        "*".to_string()
    };
    format!(
        "0 {} {} {} {} {}",
        list(&fields.minutes, 0),
        list(&fields.hours, 0),
        day_of_month,
        list(&fields.months, 0),
        day_of_week
    )
}

/// The values each of the five fields matches.
struct Fields {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days: BTreeSet<u32>,
    months: BTreeSet<u32>,
    /// Standard numbering: 0 (Sunday) to 6 (Saturday).
    weekdays: BTreeSet<u32>,
    /// The day-of-month field starts with `*`, so it does not take part in the either-or rule.
    days_star: bool,
    /// The day-of-week field starts with `*`.
    weekdays_star: bool,
}

fn parse_fields(expr: &str) -> Result<Fields, String> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    match parts.len() {
        5 => {}
        6 | 7 => {
            return Err(format!(
                "{} fields: seconds and years are not supported; use 5 fields (minute hour day-of-month month day-of-week)",
                parts.len()
            ));
        }
        n => {
            return Err(format!(
                "{n} fields: expected 5 (minute hour day-of-month month day-of-week)"
            ));
        }
    }
    Ok(Fields {
        minutes: expand(parts[0], 0, 59, &[], 0, "minute")?,
        hours: expand(parts[1], 0, 23, &[], 0, "hour")?,
        days: expand(parts[2], 1, 31, &[], 0, "day-of-month")?,
        months: expand(parts[3], 1, 12, &MONTH_NAMES, 1, "month")?,
        // 7 is Sunday too, so fold it onto 0.
        weekdays: expand(parts[4], 0, 7, &DAY_NAMES, 0, "day-of-week")?
            .into_iter()
            .map(|day| day % 7)
            .collect(),
        days_star: parts[2].starts_with('*'),
        weekdays_star: parts[4].starts_with('*'),
    })
}

/// Expands one field: comma-separated items, each `*`, `N`, `N-M`, `*/S`, `N/S` or `N-M/S`.
/// `names` (with `name_offset` for the first one) are accepted in place of numbers.
fn expand(
    text: &str,
    min: u32,
    max: u32,
    names: &[&str],
    name_offset: u32,
    field: &str,
) -> Result<BTreeSet<u32>, String> {
    let value = |token: &str| -> Result<u32, String> {
        let parsed = if !token.is_empty() && token.bytes().all(|b| b.is_ascii_digit()) {
            token.parse::<u32>().ok()
        } else {
            names
                .iter()
                .position(|name| name.eq_ignore_ascii_case(token))
                .map(|index| index as u32 + name_offset)
        };
        parsed.ok_or_else(|| format!("'{token}' is not a valid {field} value"))
    };

    let mut set = BTreeSet::new();
    for item in text.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step = step
                    .parse::<u32>()
                    .ok()
                    .filter(|step| *step > 0)
                    .ok_or_else(|| format!("invalid step '{step}' in the {field} field"))?;
                (range, step)
            }
            None => (item, 1),
        };
        let (low, high) = if range == "*" {
            (min, max)
        } else if let Some((from, to)) = range.split_once('-') {
            (value(from)?, value(to)?)
        } else {
            let single = value(range)?;
            // `N/S` runs from N to the end of the field.
            (single, if item.contains('/') { max } else { single })
        };
        if low > high {
            return Err(format!("the {field} range '{range}' runs backwards"));
        }
        if low < min || high > max {
            return Err(format!("the {field} values must be {min} to {max}"));
        }
        let mut current = low;
        while current <= high {
            set.insert(current);
            current += step;
        }
    }
    Ok(set)
}

/// A short plain-English description of a cron expression for the common shapes:
/// every N minutes, hourly, daily, weekdays, weekly, and monthly. Anything else, or an
/// invalid expression, is "custom schedule".
pub fn describe_cron(cron: &str) -> String {
    let Ok(fields) = parse_fields(cron) else {
        return CUSTOM.to_string();
    };
    let all_hours = fields.hours.len() == 24;
    let all_days = fields.days.len() == 31;
    let all_months = fields.months.len() == 12;
    let all_weekdays = fields.weekdays.len() == 7;
    let single = |set: &BTreeSet<u32>| {
        if set.len() == 1 {
            set.iter().next().copied()
        } else {
            None
        }
    };

    if all_hours
        && all_days
        && all_months
        && all_weekdays
        && let Some(step) = minute_step(&fields.minutes)
    {
        return if step == 1 {
            "every minute".to_string()
        } else {
            format!("every {step} minutes")
        };
    }
    if let Some(minute) = single(&fields.minutes)
        && all_hours
        && all_days
        && all_months
        && all_weekdays
    {
        return format!("hourly at :{minute:02}");
    }
    if let (Some(hour), Some(minute)) = (single(&fields.hours), single(&fields.minutes))
        && all_months
    {
        let time = format!("{hour:02}:{minute:02}");
        if all_days && all_weekdays {
            return format!("daily at {time}");
        }
        if all_days && fields.weekdays == BTreeSet::from([1, 2, 3, 4, 5]) {
            return format!("weekdays at {time}");
        }
        if let Some(day) = single(&fields.weekdays)
            && all_days
        {
            return format!("weekly on {} at {time}", WEEKDAY_TITLES[day as usize]);
        }
        if let Some(day) = single(&fields.days)
            && all_weekdays
        {
            return format!("monthly on day {day} at {time}");
        }
    }
    CUSTOM.to_string()
}

/// The step when the minutes are every N from :00 (`*/N`, `0,30`), so one step is known.
fn minute_step(minutes: &BTreeSet<u32>) -> Option<u32> {
    if minutes.len() == 60 {
        return Some(1);
    }
    let mut values = minutes.iter().copied();
    let first = values.next()?;
    let second = values.next()?;
    let step = second - first;
    let expected: BTreeSet<u32> = (0..60).step_by(step as usize).collect();
    (first == 0 && expected == *minutes).then_some(step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Report, Source, SourceKind};
    use chrono::TimeZone;

    /// Local time, January 2024 (no DST changes in the tests' range).
    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .expect("unambiguous local time")
    }

    fn refresh(cron: &str) -> Schedule {
        Schedule {
            name: "s".into(),
            cron: cron.into(),
            enabled: true,
            formats: vec![],
            params: vec![],
            action: ScheduleAction::Refresh { out: "out".into() },
        }
    }

    fn runs(cron: &str, after: DateTime<Local>, count: usize) -> Vec<DateTime<Local>> {
        refresh(cron).next_runs(after, count).expect("valid cron")
    }

    fn report_with(schedules: Vec<Schedule>) -> Report {
        let mut report = Report::new("r");
        report.sources = vec![Source {
            name: "s".into(),
            kind: SourceKind::File {
                path: "s.csv".into(),
                format: crate::model::FileFormat::Csv,
            },
        }];
        report.schedules = schedules;
        report
    }

    fn invalid_message(report: &Report) -> String {
        match report.validate() {
            Err(CoreError::Invalid(message)) => message,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn sunday_is_zero_or_seven_in_standard_cron() {
        // 2024-01-06 is a Saturday, 2024-01-07 a Sunday.
        let after = local(2024, 1, 6, 12, 0);
        assert_eq!(runs("* * * * 0", after, 1), vec![local(2024, 1, 7, 0, 0)]);
        assert_eq!(runs("0 8 * * 7", after, 1), vec![local(2024, 1, 7, 8, 0)]);
        assert_eq!(runs("0 8 * * sun", after, 1), vec![local(2024, 1, 7, 8, 0)]);
    }

    #[test]
    fn weekday_range_skips_the_weekend() {
        // Friday 2024-01-05 after 10:00: Monday, Tuesday, Wednesday at 09:00.
        let after = local(2024, 1, 5, 10, 0);
        assert_eq!(
            runs("0 9 * * 1-5", after, 3),
            vec![
                local(2024, 1, 8, 9, 0),
                local(2024, 1, 9, 9, 0),
                local(2024, 1, 10, 9, 0),
            ]
        );
    }

    #[test]
    fn every_fifteen_minutes_steps_from_the_hour() {
        let after = local(2024, 1, 1, 10, 7);
        assert_eq!(
            runs("*/15 * * * *", after, 3),
            vec![
                local(2024, 1, 1, 10, 15),
                local(2024, 1, 1, 10, 30),
                local(2024, 1, 1, 10, 45),
            ]
        );
    }

    #[test]
    fn day_of_month_and_day_of_week_are_either_or() {
        // Friday the 13th rule: the 13th OR any Friday. Fridays in January 2024 are the
        // 5th, 12th, 19th and 26th; the 13th is a Saturday.
        let after = local(2024, 1, 1, 0, 0);
        assert_eq!(
            runs("0 0 13 * 5", after, 3),
            vec![
                local(2024, 1, 5, 0, 0),
                local(2024, 1, 12, 0, 0),
                local(2024, 1, 13, 0, 0),
            ]
        );
    }

    #[test]
    fn names_and_lists_expand() {
        let after = local(2024, 1, 1, 0, 0);
        assert_eq!(
            runs("30 6 1 jan,mar *", after, 2),
            vec![local(2024, 1, 1, 6, 30), local(2024, 3, 1, 6, 30)]
        );
    }

    #[test]
    fn six_and_seven_field_forms_are_rejected_with_a_reason() {
        for expr in ["0 0 9 * * 1-5", "0 0 9 * * 1-5 2024"] {
            let err = refresh(expr)
                .next_runs(local(2024, 1, 1, 0, 0), 1)
                .unwrap_err();
            let text = err.to_string();
            assert!(
                text.contains("seconds and years are not supported"),
                "{text}"
            );
        }
    }

    #[test]
    fn invalid_values_and_steps_are_rejected() {
        assert!(cron_schedules("60 * * * *").is_err(), "60 * * * *");
        assert!(cron_schedules("* 24 * * *").is_err(), "* 24 * * *");
        assert!(cron_schedules("* * 0 * *").is_err(), "* * 0 * *");
        assert!(cron_schedules("* * * 13 *").is_err(), "* * * 13 *");
        assert!(cron_schedules("* * * * 8").is_err(), "* * * * 8");
        assert!(cron_schedules("*/0 * * * *").is_err(), "*/0 * * * *");
        assert!(cron_schedules("5-1 * * * *").is_err(), "5-1 * * * *");
        assert!(cron_schedules("nope * * * *").is_err(), "nope * * * *");
        assert!(cron_schedules("0 9 * * 1-5").is_ok(), "0 9 * * 1-5");
    }

    #[test]
    fn zero_count_and_no_match_are_empty() {
        assert!(runs("0 9 * * 1-5", local(2024, 1, 1, 0, 0), 0).is_empty());
        // February 30th never happens.
        assert!(runs("0 0 30 2 *", local(2024, 1, 1, 0, 0), 2).is_empty());
    }

    #[test]
    fn describe_cron_covers_the_common_shapes() {
        assert_eq!(describe_cron("*/15 * * * *"), "every 15 minutes");
        assert_eq!(describe_cron("* * * * *"), "every minute");
        assert_eq!(describe_cron("*/1 * * * *"), "every minute");
        assert_eq!(describe_cron("0 * * * *"), "hourly at :00");
        assert_eq!(describe_cron("15 * * * *"), "hourly at :15");
        assert_eq!(describe_cron("30 7 * * *"), "daily at 07:30");
        assert_eq!(describe_cron("30 7 * * 1-5"), "weekdays at 07:30");
        assert_eq!(describe_cron("0 9 * * 1"), "weekly on Monday at 09:00");
        assert_eq!(describe_cron("0 9 * * 7"), "weekly on Sunday at 09:00");
        assert_eq!(describe_cron("0 9 15 * *"), "monthly on day 15 at 09:00");
        assert_eq!(describe_cron("5/15 * * * *"), CUSTOM);
        assert_eq!(describe_cron("0 9 1,15 * *"), CUSTOM);
        assert_eq!(describe_cron("0 9 * * 1-5 *"), CUSTOM);
        assert_eq!(describe_cron("not cron"), CUSTOM);
    }

    #[test]
    fn schedules_are_omitted_from_json_when_empty_and_round_trip() {
        assert!(!Report::new("r").to_json().unwrap().contains("schedules"));

        let mut report = report_with(vec![Schedule {
            name: "weekday".into(),
            cron: "30 7 * * 1-5".into(),
            enabled: true,
            formats: vec![],
            params: vec![ScheduleParam {
                name: "region".into(),
                value: "South".into(),
            }],
            action: ScheduleAction::Refresh { out: "out".into() },
        }]);
        report.parameters = vec![Parameter::new("region", None)];
        let text = report.to_json().unwrap();
        assert!(text.contains("\"schedules\""));
        assert!(!text.contains("\"enabled\""), "enabled true is omitted");
        assert!(!text.contains("\"formats\""), "empty formats are omitted");
        let back = Report::from_json(&text).unwrap();
        assert_eq!(back, report);
        assert_eq!(back.schedules[0].effective_formats(), vec!["csv"]);
    }

    #[test]
    fn disabled_schedules_and_explicit_formats_are_written() {
        let schedule: Schedule = serde_json::from_str(
            r#"{"name":"a","cron":"0 6 * * *","enabled":false,"formats":["html","xlsx"],
                "action":{"type":"publish","target":"sftp://me@host/reports"}}"#,
        )
        .unwrap();
        assert!(!schedule.enabled);
        assert_eq!(schedule.effective_formats(), vec!["html", "xlsx"]);
        let text = serde_json::to_string(&schedule).unwrap();
        assert!(text.contains("\"enabled\":false"));
        assert!(text.contains("\"type\":\"publish\""));
    }

    #[test]
    fn default_formats_depend_on_the_action() {
        let publish = Schedule {
            action: ScheduleAction::Publish {
                target: "out".into(),
            },
            ..refresh("0 6 * * *")
        };
        assert_eq!(publish.effective_formats(), vec!["html"]);
        assert_eq!(refresh("0 6 * * *").effective_formats(), vec!["csv"]);
    }

    #[test]
    fn validation_accepts_a_good_schedule() {
        let report = report_with(vec![refresh("0 7 * * 1-5")]);
        report.validate().unwrap();
    }

    #[test]
    fn validation_rejects_bad_names_and_duplicates() {
        let mut bad = refresh("0 7 * * *");
        bad.name = "has space".into();
        assert!(invalid_message(&report_with(vec![bad])).contains("schedule name"));

        let mut a = refresh("0 7 * * *");
        a.name = "Nightly".into();
        let mut b = refresh("0 8 * * *");
        b.name = "nightly".into();
        let message = invalid_message(&report_with(vec![a, b]));
        assert!(message.contains("duplicate schedule name"), "{message}");
    }

    #[test]
    fn validation_rejects_bad_cron_formats_and_actions() {
        let six = refresh("0 0 7 * * 1-5");
        assert!(invalid_message(&report_with(vec![six])).contains("seconds and years"));

        let mut bad_format = refresh("0 7 * * *");
        bad_format.formats = vec!["docx".into()];
        assert!(invalid_message(&report_with(vec![bad_format])).contains("'docx'"));

        let mut twice = refresh("0 7 * * *");
        twice.formats = vec!["csv".into(), "csv".into()];
        assert!(invalid_message(&report_with(vec![twice])).contains("listed twice"));

        let mut svg_publish = refresh("0 7 * * *");
        svg_publish.action = ScheduleAction::Publish {
            target: "out".into(),
        };
        svg_publish.formats = vec!["svg".into()];
        assert!(invalid_message(&report_with(vec![svg_publish])).contains("'svg'"));

        let mut empty_out = refresh("0 7 * * *");
        empty_out.action = ScheduleAction::Refresh { out: "".into() };
        assert!(invalid_message(&report_with(vec![empty_out])).contains("'out'"));
    }

    #[test]
    fn validation_rejects_targets_with_passwords_but_not_user_names() {
        assert!(check_publish_target("sftp://me@host/reports").is_ok());
        assert!(check_publish_target("sftp://host:22/reports").is_ok());
        assert!(check_publish_target("s3://bucket/prefix").is_ok());
        assert!(check_publish_target("C:\\reports\\a@b").is_ok());
        assert!(check_publish_target("sftp://me:secret@host/reports").is_err());
        assert!(check_publish_target("s3://key:secret@bucket").is_err());
        assert!(check_publish_target("  ").is_err());

        let mut schedule = refresh("0 7 * * *");
        schedule.action = ScheduleAction::Publish {
            target: "https://user:pw@example.com/x".into(),
        };
        let message = invalid_message(&report_with(vec![schedule]));
        assert!(message.contains("must not contain a password"), "{message}");
    }

    #[test]
    fn validation_checks_params_against_declarations_and_types() {
        let mut report = report_with(vec![{
            let mut s = refresh("0 7 * * *");
            s.params = vec![ScheduleParam {
                name: "year".into(),
                value: "2024".into(),
            }];
            s
        }]);
        assert!(invalid_message(&report).contains("not declared"));

        report.parameters = vec![Parameter::new("year", Some(serde_json::json!(2024)))];
        report.validate().unwrap();

        report.schedules[0].params[0].value = "twenty".into();
        assert!(invalid_message(&report).contains("twenty"));

        report.schedules[0].params = vec![
            ScheduleParam {
                name: "year".into(),
                value: "2024".into(),
            },
            ScheduleParam {
                name: "year".into(),
                value: "2025".into(),
            },
        ];
        assert!(invalid_message(&report).contains("set twice"));
    }
}
