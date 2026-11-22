//! Smoke tests for dashboards: the HTML grid, shared filters per placement, the
//! CSV outputs that ignore dashboards, and exit codes for invalid dashboards.
//! Outputs go to temporary directories, never into the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const EXIT_INPUT: i32 = 3;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples folder exists")
}

fn sample_dashboard() -> PathBuf {
    examples().join("sample_dashboard.dvz")
}

fn davilize(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args(args)
        .output()
        .expect("davilize runs")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Refreshes `report` into `out` with the given formats and extra arguments.
fn refresh(report: &Path, out: &Path, formats: &str, extra: &[&str]) -> Output {
    let mut args = vec![
        "refresh",
        report.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--format",
        formats,
    ];
    args.extend_from_slice(extra);
    davilize(&args)
}

/// Writes a copy of the example report, changed by `mutate`, into `dir` (with its
/// CSV data next to it, since sources resolve against the report's folder).
fn variant(dir: &Path, mutate: impl FnOnce(&mut Value)) -> PathBuf {
    std::fs::copy(examples().join("sample.csv"), dir.join("sample.csv")).unwrap();
    let text = std::fs::read_to_string(sample_dashboard()).unwrap();
    let mut report: Value = serde_json::from_str(&text).unwrap();
    mutate(&mut report);
    let path = dir.join("variant.dvz");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).unwrap();
    path
}

/// The number of `"region":"` keys in the page's chart data: one per row of a
/// region chart, across every placement of that chart.
fn region_rows(page: &str) -> usize {
    page.matches("\"region\":\"").count()
}

#[test]
fn dashboards_lay_out_a_grid_and_apply_the_shared_region_filter() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let run = refresh(
        &sample_dashboard(),
        &out,
        "html,csv",
        &["--param", "region=North"],
    );
    assert_eq!(code(&run), 0, "{}", stderr(&run));

    let page = std::fs::read_to_string(out.join("sample_dashboard.html")).unwrap();
    assert!(page.contains("<section class=\"dashboard\" id=\"dashboard-overview\">"));
    assert!(page.contains("<section class=\"dashboard\" id=\"dashboard-by_product\">"));
    assert!(page.contains("repeat(12, 1fr); grid-auto-rows: 80px"));
    assert!(page.contains("repeat(12, 1fr); grid-auto-rows: 60px"));
    assert!(page.contains("grid-column: 1 / span 6; grid-row: 1 / span 4"));
    assert!(page.contains("grid-column: 7 / span 6; grid-row: 1 / span 4"));
    assert!(page.contains("grid-column: 1 / span 6; grid-row: 1 / span 5"));
    assert!(page.contains("<h2>By product (orders of two or more units)</h2>"));

    // The shared filter: region=North leaves only North in the region chart,
    // on both dashboards (the visual is placed twice).
    assert!(page.contains("\"region\":\"North\""));
    assert!(!page.contains("\"region\":\"South\""));
    assert_eq!(region_rows(&page), 2, "one North row per placement");

    // Per-visual CSV files are not filtered by dashboards: all four regions.
    let csv = std::fs::read_to_string(out.join("revenue_by_region.csv")).unwrap();
    assert_eq!(csv.lines().count(), 5, "header plus four regions: {csv}");
    assert!(csv.contains("South"));
}

#[test]
fn removing_the_shared_filters_shows_every_region_on_each_placement() {
    let dir = tempfile::tempdir().unwrap();
    let unfiltered = variant(dir.path(), |report| {
        for dashboard in report["dashboards"].as_array_mut().unwrap() {
            dashboard.as_object_mut().unwrap().remove("filters");
        }
    });
    let out = dir.path().join("out");
    let run = refresh(&unfiltered, &out, "html", &[]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    let page = std::fs::read_to_string(out.join("variant.html")).unwrap();
    assert!(page.contains("\"region\":\"South\""));
    // Four regions on each of the two placements of the shared visual.
    assert_eq!(region_rows(&page), 8);
}

#[test]
fn invalid_dashboards_exit_with_input_code_and_write_nothing() {
    type Mutation = fn(&mut Value);
    let cases: Vec<(&str, Mutation)> = vec![
        ("unknown visual", |r| {
            r["dashboards"][0]["items"][0]["visual"] = "missing".into();
        }),
        ("overlapping items", |r| {
            r["dashboards"][0]["items"][1]["x"] = 3.into();
        }),
        ("item past the last column", |r| {
            r["dashboards"][0]["items"][1]["x"] = 10.into();
        }),
        ("visual placed twice", |r| {
            let item = r["dashboards"][0]["items"][0].clone();
            r["dashboards"][0]["items"]
                .as_array_mut()
                .unwrap()
                .push(item);
        }),
        ("too many columns", |r| {
            r["dashboards"][0]["columns"] = 30.into();
        }),
        ("row height out of range", |r| {
            r["dashboards"][0]["row_height"] = 10.into();
        }),
        ("duplicate dashboard id", |r| {
            r["dashboards"][1]["id"] = "overview".into();
        }),
        ("aggregate-scope shared filter", |r| {
            r["dashboards"][0]["filters"][0]["scope"] = "aggregate".into();
        }),
        ("undeclared parameter in a shared filter", |r| {
            r["dashboards"][0]["filters"][0]["values"][0]["param"] = "nope".into();
        }),
        ("malformed shared filter", |r| {
            r["dashboards"][1]["filters"][1] = serde_json::json!({
                "field": "units", "op": "between", "values": [1]
            });
        }),
    ];
    for (case, mutate) in cases {
        let dir = tempfile::tempdir().unwrap();
        let report = variant(dir.path(), mutate);
        let out = dir.path().join("out");
        let result = refresh(&report, &out, "html", &[]);
        assert_eq!(code(&result), EXIT_INPUT, "{case}: {}", stderr(&result));
        assert!(!out.exists(), "{case} must fail before writing output");
    }
}

#[test]
fn dashboards_are_not_used_by_formats_other_than_html() {
    // Only html uses dashboards. csv (like svg, png, pdf and xlsx) is written per
    // visual, with no dashboard filters and no grid.
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let run = refresh(&sample_dashboard(), &out, "csv", &[]);
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    assert!(out.join("units_by_product.csv").exists());
    assert!(!out.join("sample_dashboard.html").exists());
}
