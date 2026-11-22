//! Smoke tests for report parameters on the command line: `--param` on refresh,
//! run-sql and publish, `param list`, and the exit codes for bad parameters.
//! Outputs go to temporary directories, never into the repository.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const EXIT_INPUT: i32 = 3;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .canonicalize()
        .expect("examples folder exists")
}

fn sample_params() -> PathBuf {
    examples().join("sample_params.dvz")
}

fn davilize(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args(args)
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .output()
        .expect("davilize runs")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Refreshes the sample report as CSV into `out` with extra arguments.
fn refresh_csv(out: &Path, extra: &[&str]) -> Output {
    let report = sample_params();
    let mut args = vec![
        "refresh",
        report.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--format",
        "csv",
    ];
    args.extend_from_slice(extra);
    davilize(&args)
}

#[test]
fn parameterised_refresh_writes_different_csvs_for_different_values() {
    let dir = tempfile::tempdir().unwrap();
    let one = dir.path().join("top1");
    let ten = dir.path().join("top10");
    let default = dir.path().join("defaults");
    let first = refresh_csv(&one, &["--param", "top=1"]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    assert_eq!(code(&refresh_csv(&ten, &["--param", "top=10"])), 0);
    assert_eq!(code(&refresh_csv(&default, &[])), 0);

    let read =
        |folder: &Path| std::fs::read_to_string(folder.join("revenue_by_region.csv")).unwrap();
    let top1 = read(&one);
    let top10 = read(&ten);
    assert_eq!(top1.lines().count(), 2, "header plus one row: {top1}");
    assert!(
        top10.lines().count() > 2,
        "more regions with top=10: {top10}"
    );
    assert_ne!(top1, top10);

    // Omitting a parameter uses its default (top=5, min_units=1, min_total=0).
    let explicit = dir.path().join("explicit");
    assert_eq!(
        code(&refresh_csv(
            &explicit,
            &["--param", "top=5", "--param", "min_units=1"]
        )),
        0
    );
    assert_eq!(
        read(&explicit),
        read(&default),
        "explicit defaults match the defaults"
    );

    // The region parameter changes the product table.
    let north = dir.path().join("north");
    let west = dir.path().join("west");
    assert_eq!(code(&refresh_csv(&north, &["--param", "region=North"])), 0);
    assert_eq!(code(&refresh_csv(&west, &["--param", "region=West"])), 0);
    let read_products =
        |folder: &Path| std::fs::read_to_string(folder.join("product_revenue.csv")).unwrap();
    assert_ne!(read_products(&north), read_products(&west));
}

#[test]
fn bad_parameter_values_exit_with_input_code_and_write_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let cases: Vec<Vec<&str>> = vec![
        vec!["--param", "top=99"],
        vec!["--param", "top=many"],
        vec!["--param", "region=Mars"],
        vec!["--param", "until=2026-02-30"],
        vec!["--param", "nope=1"],
        vec!["--param", "top=1", "--param", "top=2"],
        vec!["--param", "=oops"],
        vec!["--param", "top"],
    ];
    for args in cases {
        let result = refresh_csv(&out, &args);
        assert_eq!(code(&result), EXIT_INPUT, "{args:?}: {}", stderr(&result));
        assert!(!out.exists(), "{args:?} must fail before writing output");
    }
}

#[test]
fn missing_required_parameter_is_an_input_error() {
    // A report whose only parameter has no default. Its sample data is copied next to it.
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(examples().join("sample.csv"), dir.path().join("sample.csv")).unwrap();
    let report = dir.path().join("needs_region.dvz");
    std::fs::write(
        &report,
        r#"{
  "version": 1,
  "name": "Needs region",
  "sources": [{"name": "sales_csv", "type": "file", "path": "sample.csv", "format": "csv"}],
  "datasets": [{"name": "sales", "source": "sales_csv"}],
  "visuals": [{
    "id": "v", "title": "t", "dataset": "sales", "chart": "table",
    "dimensions": ["product"],
    "measures": [{"field": "revenue", "agg": "sum", "alias": "total"}],
    "filters": [{"field": "region", "op": "eq", "values": [{"param": "region"}]}]
  }],
  "parameters": [{"name": "region", "type": "string"}]
}
"#,
    )
    .unwrap();
    let out = dir.path().join("out");
    let missing = davilize(&[
        "refresh",
        report.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code(&missing), EXIT_INPUT, "{}", stderr(&missing));
    assert!(stderr(&missing).contains("missing required parameter 'region'"));

    let given = davilize(&[
        "refresh",
        report.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
        "--param",
        "region=North",
    ]);
    assert_eq!(code(&given), 0, "{}", stderr(&given));
}

#[test]
fn param_list_prints_a_table_and_json() {
    let table = davilize(&["param", "list", sample_params().to_str().unwrap()]);
    assert_eq!(code(&table), 0, "{}", stderr(&table));
    let text = String::from_utf8_lossy(&table.stdout).into_owned();
    for needle in [
        "NAME",
        "min_units",
        "integer",
        "one of \"North\", \"South\"",
        "min 1; max 50",
        "2026-03-31",
    ] {
        assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
    }

    let json = davilize(&["--json", "param", "list", sample_params().to_str().unwrap()]);
    assert_eq!(code(&json), 0, "{}", stderr(&json));
    let items: serde_json::Value = serde_json::from_slice(&json.stdout).expect("JSON on stdout");
    let items = items.as_array().unwrap();
    assert_eq!(items.len(), 5);
    let top = items.iter().find(|p| p["name"] == "top").unwrap();
    assert_eq!(top["type"], "integer");
    assert_eq!(top["default"], 5);
    assert_eq!(top["required"], false);
    assert_eq!(top["max"], 50);
    let region = items.iter().find(|p| p["name"] == "region").unwrap();
    assert_eq!(region["allowed"][0], "North");
}

#[test]
fn param_list_on_a_report_without_parameters() {
    let out = davilize(&[
        "param",
        "list",
        examples().join("sample.dvz").to_str().unwrap(),
    ]);
    assert_eq!(code(&out), 0);
    assert!(String::from_utf8_lossy(&out.stdout).contains("(no parameters)"));
}

#[test]
fn run_sql_binds_parameters_as_macros_and_rejects_them_for_data_files() {
    let ok = davilize(&[
        "run-sql",
        sample_params().to_str().unwrap(),
        r#"SELECT CAST("param_top"() AS BIGINT) AS top, "param_until"() AS until"#,
        "--format",
        "csv",
        "--param",
        "top=7",
        "--param",
        "until=2026-02-01",
    ]);
    assert_eq!(code(&ok), 0, "{}", stderr(&ok));
    let text = String::from_utf8_lossy(&ok.stdout).into_owned();
    assert!(text.contains("7,2026-02-01"), "{text}");

    let data_file = davilize(&[
        "run-sql",
        examples().join("sample.csv").to_str().unwrap(),
        "SELECT 1",
        "--param",
        "top=1",
    ]);
    assert_eq!(code(&data_file), EXIT_INPUT);

    let bad = davilize(&[
        "run-sql",
        sample_params().to_str().unwrap(),
        "SELECT 1",
        "--param",
        "top=0",
    ]);
    assert_eq!(code(&bad), EXIT_INPUT);
}

#[test]
fn publish_accepts_parameters_and_rejects_bad_ones() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("published");
    let ok = davilize(&[
        "publish",
        sample_params().to_str().unwrap(),
        "--target",
        target.to_str().unwrap(),
        "--param",
        "top=2",
    ]);
    assert_eq!(code(&ok), 0, "{}", stderr(&ok));
    assert!(target.join("index.html").exists());
    assert!(target.join("manifest.json").exists());

    let bad_target = dir.path().join("never");
    let bad = davilize(&[
        "publish",
        sample_params().to_str().unwrap(),
        "--target",
        bad_target.to_str().unwrap(),
        "--param",
        "top=500",
    ]);
    assert_eq!(code(&bad), EXIT_INPUT);
    assert!(!bad_target.exists());
}

#[test]
fn serve_param_flags_need_refresh() {
    // Clap usage error: exit code 2, and no server starts.
    let out = davilize(&["serve", "somewhere", "--param", "top=1"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn serve_refresh_applies_parameters_to_the_served_output() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let dir = tempfile::tempdir().unwrap();
    let served = dir.path().join("served");
    let mut child = Command::new(env!("CARGO_BIN_EXE_davilize"))
        .args([
            "serve",
            served.to_str().unwrap(),
            "--port",
            "0",
            "--refresh",
            sample_params().to_str().unwrap(),
            "--format",
            "html",
            "--param",
            "top=2",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("davilize serve starts");

    // Wait for the listening line, then stop the server. Its first generation is
    // written before it listens, so the page must exist by then.
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let mut listening = false;
    for line in lines.by_ref() {
        let line = line.unwrap();
        if line.contains("serving") {
            listening = true;
            break;
        }
        if line.contains("[error]") {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(listening, "server did not start");

    let generation = std::fs::read_dir(&served)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.join("index.html").exists())
        .expect("the first generation has an index.html");
    let page = std::fs::read_to_string(generation.join("index.html")).unwrap();
    assert!(page.contains("Revenue by region (parameterised)"));
}
