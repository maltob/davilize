//! Integration tests: aggregate (HAVING) and `between` filters, calculated fields
//! and bound parameters, run against the sample data in examples/.

use std::path::{Path, PathBuf};

use duckdb::arrow::array::{Array, Float64Array, Int64Array, StringArray};
use duckdb::arrow::record_batch::RecordBatch;
use dv_core::{
    Aggregate, ChartType, Encodings, Filter, FilterOp, FilterScope, Limit, Measure, Report, Visual,
    bind_visual, resolve_parameters,
};
use dv_engine::{Engine, EngineError};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// Loads an example report and registers its sources and datasets.
fn load(name: &str) -> (Report, Engine) {
    let path = examples_dir().join(name);
    let report = Report::load(&path).expect("example report loads");
    let engine = Engine::new().unwrap();
    engine
        .register_report(&report, &examples_dir())
        .expect("example report registers");
    (report, engine)
}

fn rows(batches: &[RecordBatch]) -> usize {
    batches.iter().map(|b| b.num_rows()).sum()
}

fn floats(batches: &[RecordBatch], column: &str) -> Vec<f64> {
    let mut out = Vec::new();
    for batch in batches {
        let array = batch
            .column_by_name(column)
            .unwrap_or_else(|| panic!("column {column}"));
        let array = array
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap_or_else(|| panic!("{column} is DOUBLE"));
        out.extend((0..array.len()).map(|i| array.value(i)));
    }
    out
}

fn strings(batches: &[RecordBatch], column: &str) -> Vec<String> {
    let mut out = Vec::new();
    for batch in batches {
        let array = batch
            .column_by_name(column)
            .unwrap_or_else(|| panic!("column {column}"))
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap_or_else(|| panic!("{column} is VARCHAR"))
            .clone();
        out.extend((0..array.len()).map(|i| array.value(i).to_string()));
    }
    out
}

fn visual<'a>(report: &'a Report, id: &str) -> &'a Visual {
    report
        .visuals
        .iter()
        .find(|v| v.id == id)
        .unwrap_or_else(|| panic!("visual {id}"))
}

/// Runs a visual with the report's parameters bound from `supplied`.
fn run_bound(
    report: &Report,
    engine: &Engine,
    id: &str,
    supplied: &[(&str, &str)],
) -> Result<Vec<RecordBatch>, EngineError> {
    let supplied: Vec<(String, String)> = supplied
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    let values = resolve_parameters(&report.parameters, &supplied).unwrap();
    let bound = bind_visual(visual(report, id), &values).unwrap();
    engine.run_visual(&bound)
}

#[test]
fn aggregate_filter_keeps_only_groups_above_the_threshold() {
    let (report, engine) = load("sample.dvz");
    let mut v = visual(&report, "product_region_mix").clone();
    // Groups are product x region; keep those with more than 100 total revenue.
    v.filters.push(Filter {
        field: "total_revenue".into(),
        op: FilterOp::Gt,
        values: vec![serde_json::json!(100)],
        scope: FilterScope::Aggregate,
    });
    let sql = engine.compile(&v).unwrap();
    assert!(
        sql.contains(r#"HAVING SUM("revenue") > 100"#),
        "aggregate filters use the aggregate expression: {sql}"
    );
    let batches = engine.run_visual(&v).unwrap();
    let totals = floats(&batches, "total_revenue");
    assert!(!totals.is_empty(), "some groups remain");
    assert!(totals.iter().all(|t| *t > 100.0), "{totals:?}");

    // Without the filter there are more groups, and some are at or below 100.
    let all = engine
        .run_visual(visual(&report, "product_region_mix"))
        .unwrap();
    assert!(rows(&all) > rows(&batches));
    assert!(floats(&all, "total_revenue").iter().any(|t| *t <= 100.0));
}

#[test]
fn between_filters_are_inclusive_on_rows_and_aggregates() {
    let (report, engine) = load("sample.dvz");
    let mut v = visual(&report, "product_region_mix").clone();
    v.filters = vec![Filter {
        field: "units".into(),
        op: FilterOp::Between,
        values: vec![serde_json::json!(2), serde_json::json!(5)],
        scope: FilterScope::Row,
    }];
    let batches = engine.run_visual(&v).unwrap();
    assert!(rows(&batches) > 0);

    // Row-level check: every raw row in range has units 2..=5.
    let raw = engine
        .query(r#"SELECT min(units) AS lo, max(units) AS hi FROM "sales_csv" WHERE units BETWEEN 2 AND 5"#)
        .unwrap();
    let lo = raw[0]
        .column_by_name("lo")
        .unwrap()
        .as_any()
        .downcast_ref::<Int64Array>();
    let hi = raw[0]
        .column_by_name("hi")
        .unwrap()
        .as_any()
        .downcast_ref::<Int64Array>();
    assert_eq!(lo.unwrap().value(0), 2);
    assert_eq!(hi.unwrap().value(0), 5);

    // Aggregate between, inclusive at both ends.
    let mut v = visual(&report, "product_region_mix").clone();
    v.filters = vec![Filter {
        field: "total_revenue".into(),
        op: FilterOp::Between,
        values: vec![serde_json::json!(100), serde_json::json!(400)],
        scope: FilterScope::Aggregate,
    }];
    let batches = engine.run_visual(&v).unwrap();
    let totals = floats(&batches, "total_revenue");
    assert!(
        totals.iter().all(|t| (100.0..=400.0).contains(t)),
        "{totals:?}"
    );
}

#[test]
fn calculated_fields_are_usable_in_visuals_and_filters() {
    let (report, engine) = load("sample_params.dvz");
    let batches = run_bound(&report, &engine, "big_ticket_units", &[]).unwrap();
    assert_eq!(strings(&batches, "big_ticket"), ["no", "yes"]);
    assert_eq!(rows(&batches), 2);

    // Filter on a calculated column, row scope.
    let mut v = visual(&report, "big_ticket_units").clone();
    v.filters.push(Filter {
        field: "big_ticket".into(),
        op: FilterOp::Eq,
        values: vec![serde_json::json!("yes")],
        scope: FilterScope::Row,
    });
    let batches = engine.run_visual(&v).unwrap();
    assert_eq!(strings(&batches, "big_ticket"), ["yes"]);
}

#[test]
fn parameters_change_the_results() {
    let (report, engine) = load("sample_params.dvz");

    // `top` limits the number of regions.
    let one = run_bound(&report, &engine, "revenue_by_region", &[("top", "1")]).unwrap();
    let all = run_bound(&report, &engine, "revenue_by_region", &[("top", "10")]).unwrap();
    assert_eq!(rows(&one), 1);
    assert!(rows(&all) > 1);

    // `min_units` drops rows before grouping: a high threshold empties the groups.
    let none = run_bound(
        &report,
        &engine,
        "revenue_by_region",
        &[("min_units", "100")],
    )
    .unwrap();
    assert_eq!(rows(&none), 0);

    // `min_total` is an aggregate (HAVING) threshold.
    let high = run_bound(
        &report,
        &engine,
        "revenue_by_region",
        &[("min_total", "400")],
    )
    .unwrap();
    assert!(floats(&high, "total_revenue").iter().all(|t| *t > 400.0));

    // `region` and `until` change the product table.
    let north = run_bound(&report, &engine, "product_revenue", &[("region", "North")]).unwrap();
    let west = run_bound(&report, &engine, "product_revenue", &[("region", "West")]).unwrap();
    assert_ne!(
        floats(&north, "revenue_total"),
        floats(&west, "revenue_total")
    );
    let early = run_bound(
        &report,
        &engine,
        "product_revenue",
        &[("region", "North"), ("until", "2026-01-31")],
    )
    .unwrap();
    let late = run_bound(&report, &engine, "product_revenue", &[("region", "North")]).unwrap();
    let sum = |b: &[RecordBatch]| floats(b, "revenue_total").iter().sum::<f64>();
    assert!(
        sum(&early) < sum(&late),
        "a later end date includes more orders"
    );
}

#[test]
fn parameter_values_are_validated_before_any_query_runs() {
    let (report, _engine) = load("sample_params.dvz");
    let bad = |pairs: &[(&str, &str)]| {
        let supplied: Vec<(String, String)> = pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect();
        resolve_parameters(&report.parameters, &supplied)
    };
    assert!(bad(&[("region", "Mars")]).is_err(), "not in allowed");
    assert!(bad(&[("top", "51")]).is_err(), "above max");
    assert!(bad(&[("top", "0")]).is_err(), "below min");
    assert!(bad(&[("until", "2026-02-30")]).is_err(), "no such day");
    assert!(bad(&[("min_units", "1.5")]).is_err(), "not an integer");
    assert!(bad(&[("nope", "1")]).is_err(), "unknown name");
    assert!(bad(&[("region", "West"), ("top", "3")]).is_ok());
}

#[test]
fn injection_attempts_in_parameters_stay_literals() {
    let (report, engine) = load("sample_params.dvz");
    // Region is an allowed-values parameter, so this is refused before SQL is built.
    assert!(
        resolve_parameters(
            &report.parameters,
            &[("region".into(), "North' OR '1'='1".into())]
        )
        .is_err()
    );

    // A string filter value with quotes is rendered as a literal, not as SQL.
    let mut v: Visual = visual(&report, "product_revenue").clone();
    v.filters = vec![Filter {
        field: "product".into(),
        op: FilterOp::Eq,
        values: vec![serde_json::json!("x'; DROP TABLE sales; --")],
        scope: FilterScope::Row,
    }];
    let batches = engine.run_visual(&v).unwrap();
    assert_eq!(rows(&batches), 0);
    assert!(engine.query(r#"SELECT count(*) FROM "sales""#).is_ok());
}

#[test]
fn limit_parameters_bind_to_rows() {
    let (report, _engine) = load("sample_params.dvz");
    let values = resolve_parameters(&report.parameters, &[("top".into(), "3".into())]).unwrap();
    let bound = bind_visual(visual(&report, "revenue_by_region"), &values).unwrap();
    assert_eq!(bound.limit, Some(Limit::Rows(3)));
}

#[test]
fn programmatic_visual_with_between_and_aggregate_compiles_end_to_end() {
    let (_report, engine) = load("sample.dvz");
    let v = Visual {
        id: "adhoc".into(),
        title: "Ad hoc".into(),
        dataset: "sales".into(),
        chart: ChartType::Table,
        encodings: Encodings::default(),
        dimensions: vec!["region".into()],
        measures: vec![Measure {
            field: "revenue".into(),
            agg: Aggregate::Sum,
            alias: Some("total".into()),
        }],
        filters: vec![Filter {
            field: "total".into(),
            op: FilterOp::Between,
            values: vec![serde_json::json!(0), serde_json::json!(10_000)],
            scope: FilterScope::Aggregate,
        }],
        sort: None,
        limit: Some(Limit::Rows(10)),
    };
    let batches = engine.run_visual(&v).unwrap();
    assert!(rows(&batches) > 0);
}
