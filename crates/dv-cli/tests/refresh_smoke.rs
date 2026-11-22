//! Smoke test: `davilize refresh` over examples/sample.dvz in every output format.
//! Outputs go to a temporary directory, never into the repository.

use std::path::{Path, PathBuf};
use std::process::Command;

fn sample_report() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/sample.dvz")
        .canonicalize()
        .expect("examples/sample.dvz exists")
}

fn refresh(out: &Path, formats: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .arg("refresh")
        .arg(sample_report())
        .arg("--out")
        .arg(out)
        .arg("--format")
        .arg(formats)
        .output()
        .expect("davilize runs")
}

#[test]
fn refresh_writes_every_format_with_valid_signatures() {
    let dir = tempfile::tempdir().unwrap();
    let out = refresh(dir.path(), "csv,parquet,html,svg,png,pdf,xlsx");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let read = |name: &str| {
        std::fs::read(dir.path().join(name)).unwrap_or_else(|_| panic!("missing {name}"))
    };

    let png = read("revenue_by_region.png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&read("revenue_by_region.pdf")[..5], b"%PDF-");
    let svg = String::from_utf8(read("revenue_by_region.svg")).unwrap();
    assert!(svg.starts_with("<svg "));

    let html = String::from_utf8(read("sample.html")).unwrap();
    assert!(html.starts_with("<!doctype html>"));
    assert_eq!(html.matches("vega-embed@7.3.0").count(), 1);
    assert!(html.contains("Revenue by region"));

    let xlsx = read("sample.xlsx");
    assert_eq!(&xlsx[..4], b"PK\x03\x04");

    // The table visual now has a native renderer, so it is written like any chart.
    let table_svg = String::from_utf8(read("customer_table.svg")).unwrap();
    assert!(table_svg.contains("Orders by customer") && table_svg.contains(">customer</text>"));
    assert_eq!(&read("customer_table.png")[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&read("customer_table.pdf")[..5], b"%PDF-");
    assert!(dir.path().join("customer_table.csv").exists());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("skipped svg/png/pdf"));
}

#[test]
fn unknown_format_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = refresh(dir.path(), "docx");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn offline_requires_a_vendor_dir() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_davilize"))
        .arg("refresh")
        .arg(sample_report())
        .arg("--out")
        .arg(dir.path())
        .arg("--format")
        .arg("html")
        .arg("--offline")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
