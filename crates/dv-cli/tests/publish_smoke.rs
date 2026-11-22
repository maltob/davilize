//! Smoke tests for `davilize publish` against a local folder, and its exit codes.
//! Outputs go to temporary directories, never into the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn sample_report() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sample.dvz")
        .canonicalize()
        .expect("examples/sample.dvz exists")
}

fn davilize(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args(args)
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .output()
        .expect("davilize runs")
}

#[test]
fn publish_to_a_local_folder_writes_index_manifest_and_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("published");
    let report = sample_report();
    let out = davilize(&[
        "publish",
        report.to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
        "--format",
        "html,csv,png",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let index = std::fs::read_to_string(target.join("index.html")).unwrap();
    assert!(index.starts_with("<!doctype html>"));
    assert!(index.contains("Revenue by region"));
    assert!(target.join("revenue_by_region.csv").exists());
    assert!(target.join("revenue_by_region.png").exists());

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(target.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["format"], "davilize-publish-manifest");
    assert_eq!(manifest["format_version"], 1);
    assert_eq!(manifest["report"]["name"], "Sample sales");
    assert_eq!(manifest["report"]["version"], 1);
    let generated = manifest["generated_at"].as_str().unwrap();
    assert!(
        generated.ends_with('Z') && generated.len() == 20,
        "{generated}"
    );

    let files = manifest["files"].as_array().unwrap();
    let index_entry = files.iter().find(|f| f["name"] == "index.html").unwrap();
    assert_eq!(index_entry["size"].as_u64().unwrap(), index.len() as u64);
    let digest = index_entry["sha256"].as_str().unwrap();
    assert_eq!(digest.len(), 64);
    assert!(files.iter().all(|f| f["name"] != "manifest.json"));

    // The log says what was uploaded, and index.html goes last.
    let stderr = String::from_utf8_lossy(&out.stderr);
    let uploaded: Vec<&str> = stderr
        .lines()
        .filter_map(|l| l.strip_prefix("[info] uploaded "))
        .filter_map(|l| l.split(' ').next())
        .collect();
    assert_eq!(uploaded.last(), Some(&"index.html"), "{stderr}");
    assert!(!stderr.contains("sftp") && !stderr.contains("secret"));
}

#[test]
fn dry_run_lists_files_and_uploads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("never-created");
    let out = davilize(&[
        "publish",
        sample_report().to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
        "--dry-run",
    ]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("Dry run:"), "{stdout}");
    // index.html is listed last, as it is uploaded last.
    let last = stdout.lines().last().unwrap_or_default();
    assert!(last.contains(" index.html  sha256:"), "{stdout}");
    assert!(stdout.contains(" manifest.json  sha256:"), "{stdout}");
    assert!(!target.exists(), "dry run must not create the target");
}

#[test]
fn a_target_with_a_password_is_an_input_error() {
    let out = davilize(&[
        "publish",
        sample_report().to_str().unwrap(),
        "--target",
        "sftp://deploy:hunter2@example.com/srv",
    ]);
    assert_eq!(out.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("hunter2"), "{stderr}");
}

#[test]
fn incomplete_s3_credentials_are_a_publish_error_with_exit_code_6() {
    let out = Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args([
            "publish",
            sample_report().to_str().unwrap(),
            "--target",
            "s3://example-bucket/prefix",
            "--dry-run",
        ])
        .output()
        .unwrap();
    // A dry run does not build the operator, so it succeeds without credentials.
    assert!(out.status.success());

    let out = Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args([
            "publish",
            sample_report().to_str().unwrap(),
            "--target",
            "s3://example-bucket/prefix",
        ])
        .env("AWS_ACCESS_KEY_ID", "AKIAFAKEKEYVALUE")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(6));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("must be set together"), "{stderr}");
    assert!(!stderr.contains("AKIAFAKEKEYVALUE"), "{stderr}");
}

#[test]
fn offline_publish_requires_a_vendor_dir() {
    let out = davilize(&[
        "publish",
        sample_report().to_str().unwrap(),
        "--target",
        "out",
        "--offline",
    ]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn serve_refuses_a_missing_folder_with_input_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope");
    let out = davilize(&["serve", missing.to_str().unwrap(), "--port", "0"]);
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn sftp_without_a_user_is_exit_code_6_and_never_connects() {
    let out = davilize(&[
        "publish",
        sample_report().to_str().unwrap(),
        "--target",
        "sftp://example.invalid/srv",
    ]);
    assert_eq!(out.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&out.stderr).contains("DAVILIZE_SFTP_USER"));
}
