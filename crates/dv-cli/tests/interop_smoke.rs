//! Smoke test: export a report to Tableau and Power BI formats, import it back,
//! and refresh the imported report with the engine.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

fn example(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(name)
        .canonicalize()
        .unwrap()
}

fn davilize(args: &[&OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args(args)
        .output()
        .expect("davilize runs")
}

fn os(p: &Path) -> &OsStr {
    p.as_os_str()
}

fn round_trip(format: &str, file_name: &str) {
    let dir = tempfile::tempdir().unwrap();
    let exported = dir.path().join(file_name);
    let report = example("sample_joined.dvz");
    let out = davilize(&[
        "export".as_ref(),
        os(&report),
        "--format".as_ref(),
        format.as_ref(),
        "--out".as_ref(),
        os(&exported),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(exported.exists());

    let back = dir.path().join("back").join("back.dvz");
    std::fs::create_dir_all(back.parent().unwrap()).unwrap();
    let out = davilize(&[
        "import".as_ref(),
        os(&exported),
        "--out".as_ref(),
        os(&back),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let refreshed = dir.path().join("refreshed");
    let out = davilize(&[
        "refresh".as_ref(),
        os(&back),
        "--out".as_ref(),
        os(&refreshed),
        "--format".as_ref(),
        "csv".as_ref(),
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let csvs: Vec<_> = std::fs::read_dir(&refreshed)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "csv"))
        .collect();
    assert!(!csvs.is_empty());
    let text = std::fs::read_to_string(csvs[0].path()).unwrap();
    assert!(
        text.contains("Ann"),
        "joined manager column missing: {text}"
    );
}

#[test]
fn tableau_workbook_round_trips_and_refreshes() {
    round_trip("twb", "wb.twb");
}

#[test]
fn packaged_tableau_workbook_round_trips_and_refreshes() {
    round_trip("twbx", "wb.twbx");
}

#[test]
fn power_bi_template_round_trips_and_refreshes() {
    round_trip("pbit", "wb.pbit");
}

#[test]
fn power_bi_project_round_trips_and_refreshes() {
    round_trip("pbip", "project");
}

#[test]
fn importing_something_else_fails_with_the_input_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let junk = dir.path().join("x.txt");
    std::fs::write(&junk, "hello").unwrap();
    let out = davilize(&[
        "import".as_ref(),
        os(&junk),
        "--out".as_ref(),
        os(&dir.path().join("o.dvz")),
    ]);
    assert_eq!(out.status.code(), Some(3));
}
