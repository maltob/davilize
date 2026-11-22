//! Generates a 10M-row Parquet file with DuckDB itself, then runs a compiled
//! aggregation over it through the engine.

use std::time::Instant;

use duckdb::arrow::array::{Float64Array, Int64Array};
use dv_core::{
    Aggregate, ChartType, Encodings, Filter, FilterOp, Measure, Sort, SortDirection, Visual,
};
use dv_engine::{Engine, OutputFormat};

const ROWS: i64 = 10_000_000;

#[test]
fn ten_million_row_parquet_aggregation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.parquet");

    let engine = Engine::new().unwrap();
    let gen_start = Instant::now();
    engine
        .execute(&format!(
            "COPY (SELECT i AS id, i % 100 AS category, (i % 1000)::DOUBLE AS value \
             FROM range({ROWS}) t(i)) TO '{}' (FORMAT PARQUET)",
            path.to_string_lossy().replace('\'', "''")
        ))
        .unwrap();
    eprintln!("generated {ROWS} rows in {:?}", gen_start.elapsed());

    engine.register_file("big", &path).unwrap();

    let visual = Visual {
        id: "by_category".into(),
        title: "Total value by category".into(),
        dataset: "big".into(),
        chart: ChartType::Bar,
        encodings: Encodings {
            x: Some("category".into()),
            y: Some("total".into()),
            color: None,
            size: None,
        },
        dimensions: vec!["category".into()],
        measures: vec![
            Measure {
                field: "value".into(),
                agg: Aggregate::Sum,
                alias: Some("total".into()),
            },
            Measure {
                field: "id".into(),
                agg: Aggregate::Count,
                alias: Some("n".into()),
            },
        ],
        filters: vec![Filter {
            field: "id".into(),
            op: FilterOp::Ge,
            values: vec![serde_json::json!(0)],
            scope: dv_core::FilterScope::Row,
        }],
        sort: Some(Sort {
            by: "total".into(),
            direction: SortDirection::Desc,
        }),
        limit: None,
    };

    let query_start = Instant::now();
    let batches = engine.run_visual(&visual).unwrap();
    let elapsed = query_start.elapsed();
    eprintln!("aggregation over {ROWS} rows took {elapsed:?}");

    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(rows, 100, "one row per category");

    let mut total_count = 0i64;
    let mut totals_in_order: Vec<f64> = Vec::new();
    for batch in &batches {
        let totals = batch
            .column_by_name("total")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let counts = batch
            .column_by_name("n")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        for row in 0..batch.num_rows() {
            assert_eq!(counts.value(row), ROWS / 100, "each category has 100k rows");
            total_count += counts.value(row);
            totals_in_order.push(totals.value(row));
        }
    }
    assert_eq!(total_count, ROWS);
    // Each block of 1000 ids contributes sum(0..999) = 499500; there are 10_000 blocks
    // and every category receives 100 of them.
    let grand_total: f64 = totals_in_order.iter().sum();
    assert_eq!(grand_total, 10_000.0 * 499_500.0);
    // ORDER BY total DESC is applied across the whole result.
    assert!(totals_in_order.windows(2).all(|w| w[0] >= w[1]));

    // Profiling a large parquet relation works too.
    let profile = engine.profile("big").unwrap();
    assert!(profile.iter().map(|b| b.num_rows()).sum::<usize>() >= 3);

    // Export the aggregate to CSV and Parquet via DuckDB COPY.
    let sql = dv_engine::compile_visual(&visual).unwrap();
    let csv_out = dir.path().join("by_category.csv");
    let written = engine.export(&sql, &csv_out, OutputFormat::Csv).unwrap();
    assert_eq!(written, 100);

    assert!(
        elapsed.as_secs() < 30,
        "aggregation unexpectedly slow: {elapsed:?}"
    );
}
