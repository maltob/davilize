//! Engine tests for calculated fields and parameter macros: column layering,
//! name rules, per-field errors, and the hostile-expression set.

use std::fs;
use std::path::{Path, PathBuf};

use duckdb::arrow::array::{Array, Int64Array};
use dv_core::{
    Aggregate, CalculatedField, ChartType, Dataset, Encodings, FileFormat, Filter, FilterOp,
    FilterScope, Measure, ParamType, ParamValues, Parameter, Report, Source, SourceKind, Visual,
};

use crate::{Engine, EngineError};

const SAMPLE_CSV: &str =
    "region,units,revenue\nNorth,3,30.0\nSouth,5,55.5\nNorth,2,20.0\nWest,7,70.0\n";

fn write_sample(dir: &Path) -> PathBuf {
    let path = dir.join("sample.csv");
    fs::write(&path, SAMPLE_CSV).unwrap();
    path
}

/// An engine with the sample CSV registered as the source `sales`.
fn engine_with_sales(dir: &Path) -> Engine {
    let path = write_sample(dir);
    let engine = Engine::new().unwrap();
    engine
        .register_source(
            &Source {
                name: "sales".into(),
                kind: SourceKind::File {
                    path,
                    format: FileFormat::Csv,
                },
            },
            Path::new(""),
        )
        .unwrap();
    engine
}

fn calc(name: &str, expression: &str) -> CalculatedField {
    CalculatedField {
        name: name.into(),
        expression: expression.into(),
        description: None,
    }
}

fn dataset_with(name: &str, source: &str, fields: Vec<CalculatedField>) -> Dataset {
    Dataset {
        name: name.into(),
        source: source.into(),
        sql: None,
        joins: Vec::new(),
        calculated_fields: fields,
    }
}

fn scalar_i64(engine: &Engine, sql: &str) -> i64 {
    let batches = engine.query(sql).unwrap();
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .expect("BIGINT result")
        .value(0)
}

#[test]
fn calculated_fields_are_columns_and_later_fields_see_earlier_ones() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sales(dir.path());
    engine
        .register_dataset(&dataset_with(
            "enriched",
            "sales",
            vec![
                calc("double_units", "units * 2"),
                calc(
                    "big",
                    "CASE WHEN double_units >= 10 THEN 'yes' ELSE 'no' END",
                ),
            ],
        ))
        .unwrap();

    let schema = engine.schema("enriched").unwrap();
    let names: Vec<&str> = schema.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["region", "units", "revenue", "double_units", "big"]);
    let flags: Vec<bool> = schema.iter().map(|c| c.calculated).collect();
    assert_eq!(flags, [false, false, false, true, true]);
    assert!(
        engine
            .schema("sales")
            .unwrap()
            .iter()
            .all(|c| !c.calculated)
    );

    // units 3, 5, 2, 7 -> doubled 6, 10, 4, 14 -> "yes" for the last two rows.
    assert_eq!(
        scalar_i64(
            &engine,
            r#"SELECT count(*) FROM "enriched" WHERE "big" = 'yes'"#
        ),
        2
    );
    assert_eq!(
        scalar_i64(
            &engine,
            r#"SELECT count(*) FROM "enriched" WHERE "double_units" = 14"#
        ),
        1
    );
}

#[test]
fn calculated_fields_layer_over_dataset_sql_and_appear_in_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_sample(dir.path());
    let mut report = Report::new("r");
    report.sources.push(Source {
        name: "raw".into(),
        kind: SourceKind::File {
            path,
            format: FileFormat::Csv,
        },
    });
    report.datasets.push(Dataset {
        name: "big_orders".into(),
        source: "raw".into(),
        sql: Some("SELECT * FROM raw WHERE units >= 5".into()),
        joins: Vec::new(),
        calculated_fields: vec![calc("per_unit", "revenue / units")],
    });
    let engine = Engine::new().unwrap();
    engine.register_report(&report, Path::new("")).unwrap();
    assert_eq!(
        scalar_i64(&engine, r#"SELECT count(*) FROM "big_orders""#),
        2
    );
    let profile = engine.profile("big_orders").unwrap();
    let rows: usize = profile.iter().map(|b| b.num_rows()).sum();
    assert_eq!(
        rows, 4,
        "one profile row per column, including the calculated one"
    );
}

#[test]
fn calculated_names_must_not_collide_with_columns_or_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sales(dir.path());

    // The same name as a real column, in any case.
    let err = engine
        .register_dataset(&dataset_with(
            "d1",
            "sales",
            vec![calc("UNITS", "units + 1")],
        ))
        .unwrap_err();
    assert!(err.to_string().contains("same name as a column"), "{err}");
    assert!(engine.schema("d1").is_err(), "no view is left behind");

    // Duplicate calculated names, ignoring case.
    let err = engine
        .register_dataset(&dataset_with(
            "d2",
            "sales",
            vec![calc("x", "units"), calc("X", "units")],
        ))
        .unwrap_err();
    assert!(err.to_string().contains("twice"), "{err}");

    // Names must be identifiers.
    let err = engine
        .register_dataset(&dataset_with(
            "d3",
            "sales",
            vec![calc("bad name", "units")],
        ))
        .unwrap_err();
    assert!(matches!(err, EngineError::InvalidSpec(_)), "{err}");
}

#[test]
fn calculated_field_errors_name_the_field() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sales(dir.path());

    let err = engine
        .register_dataset(&dataset_with(
            "d",
            "sales",
            vec![calc("x", "no_such_column * 2")],
        ))
        .unwrap_err()
        .to_string();
    assert!(err.contains("calculated field 'x'"), "{err}");

    // A field cannot use a field that is defined after it.
    let err = engine
        .register_dataset(&dataset_with(
            "d",
            "sales",
            vec![calc("a", "b + 1"), calc("b", "units")],
        ))
        .unwrap_err()
        .to_string();
    assert!(err.contains("calculated field 'a'"), "{err}");
}

/// Expressions that must never run: file readers, environment and settings
/// access, subqueries, statements smuggled in with `;`, comments, attach, copy.
const HOSTILE_EXPRESSIONS: &[&str] = &[
    "read_csv('/etc/passwd')",
    "read_csv_auto('sample.csv')",
    "read_parquet('x.parquet')",
    "read_json('x.json')",
    "glob('/*')",
    "getenv('HOME')",
    "current_setting('home_directory')",
    "(SELECT count(*) FROM sales)",
    "units + (SELECT 1)",
    "1; DROP TABLE sales",
    "units); DROP TABLE sales; SELECT (1",
    "units -- and the rest is ignored",
    "units /* hidden */ + 0",
    "1) FROM read_csv('x') --",
    "ATTACH '/tmp/evil.db' AS evil",
    "COPY sales TO '/tmp/out.csv'",
    "units FROM read_csv('x')",
    "sum(units)",
    "1 UNION ALL SELECT 2",
];

#[test]
fn hostile_expressions_are_refused_and_leave_no_view_or_database() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sales(dir.path());
    let databases_before = scalar_rows(&engine, "SELECT * FROM duckdb_databases()");

    for (i, expression) in HOSTILE_EXPRESSIONS.iter().enumerate() {
        let name = format!("evil_{i}");
        let err = engine
            .register_dataset(&dataset_with(&name, "sales", vec![calc("f", expression)]))
            .expect_err(expression);
        assert!(
            matches!(err, EngineError::InvalidSpec(_)),
            "{expression}: {err}"
        );
        assert!(
            err.to_string().contains("calculated field 'f'"),
            "{expression}: {err}"
        );
        assert!(engine.schema(&name).is_err(), "{expression} left a view");
    }
    assert_eq!(
        scalar_rows(&engine, "SELECT * FROM duckdb_databases()"),
        databases_before,
        "no database was attached"
    );
}

fn scalar_rows(engine: &Engine, sql: &str) -> usize {
    engine
        .query(sql)
        .unwrap()
        .iter()
        .map(|b| b.num_rows())
        .sum()
}

#[test]
fn parameter_macros_expose_typed_bound_values_to_run_sql() {
    let mut year = Parameter::new("year", Some(serde_json::json!(2024)));
    year.kind = Some(ParamType::Integer);
    let mut region = Parameter::new("region", None);
    region.kind = Some(ParamType::String);
    let mut day = Parameter::new("day", None);
    day.kind = Some(ParamType::Date);
    let params = vec![year, region, day];

    let mut values = ParamValues::new();
    values.insert("year".into(), serde_json::json!(2023));
    values.insert("region".into(), serde_json::json!("O'Brien"));
    values.insert("day".into(), serde_json::json!("2024-02-29"));
    let engine = Engine::new().unwrap();
    engine.define_parameter_macros(&params, &values).unwrap();

    assert_eq!(
        scalar_i64(&engine, r#"SELECT CAST("param_year"() AS BIGINT)"#),
        2023
    );
    let batches = engine
        .query(r#"SELECT "param_region"() AS r, "param_day"() AS d"#)
        .unwrap();
    let mut text = String::new();
    for batch in &batches {
        for column in batch.columns() {
            for row in 0..batch.num_rows() {
                text.push_str(
                    &duckdb::arrow::util::display::array_value_to_string(column, row).unwrap(),
                );
                text.push('|');
            }
        }
    }
    assert!(text.contains("O'Brien|"), "{text}");
    assert!(text.contains("2024-02-29|"), "{text}");
}

#[test]
fn compile_uses_the_dataset_columns_including_calculated_ones() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sales(dir.path());
    engine
        .register_dataset(&dataset_with(
            "enriched",
            "sales",
            vec![calc("x", "units * 2")],
        ))
        .unwrap();

    let mut visual = Visual {
        id: "v".into(),
        title: "t".into(),
        dataset: "enriched".into(),
        chart: ChartType::Table,
        encodings: Encodings::default(),
        dimensions: vec!["region".into()],
        measures: vec![Measure {
            field: "x".into(),
            agg: Aggregate::Sum,
            alias: Some("total_x".into()),
        }],
        filters: vec![Filter {
            field: "x".into(),
            op: FilterOp::Gt,
            values: vec![serde_json::json!(4)],
            scope: FilterScope::Row,
        }],
        sort: None,
        limit: None,
    };
    let sql = engine.compile(&visual).unwrap();
    assert!(sql.contains(r#"WHERE "x" > 4"#), "{sql}");
    assert!(engine.run_visual(&visual).is_ok());

    visual.dataset = "missing".into();
    assert!(engine.compile(&visual).is_err());
}
