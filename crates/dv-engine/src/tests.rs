use std::fs;
use std::path::Path;

use duckdb::arrow::array::{Array, Int64Array};
use dv_core::{Aggregate, ChartType, Encodings, Measure, Report, Source, SourceKind, Visual};

use crate::sql;
use crate::{Engine, EngineError, OutputFormat, infer_format};

const SAMPLE_CSV: &str =
    "region,units,revenue\nNorth,3,30.0\nSouth,5,55.5\nNorth,2,20.0\nWest,7,70.0\n";

fn write_sample(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("sample.csv");
    fs::write(&path, SAMPLE_CSV).unwrap();
    path
}

fn total_rows(batches: &[duckdb::arrow::record_batch::RecordBatch]) -> usize {
    batches.iter().map(|b| b.num_rows()).sum()
}

fn csv_source(name: &str, path: &Path) -> Source {
    Source {
        name: name.into(),
        kind: SourceKind::File {
            path: path.to_path_buf(),
            format: dv_core::FileFormat::Csv,
        },
    }
}

#[test]
fn csv_schema_and_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_sample(dir.path());
    let engine = Engine::new().unwrap();
    engine
        .register_source(&csv_source("sales", &path), Path::new(""))
        .unwrap();

    let schema = engine.schema("sales").unwrap();
    let names: Vec<&str> = schema.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["region", "units", "revenue"]);
    assert_eq!(schema[1].data_type, "BIGINT");
    assert_eq!(schema[2].data_type, "DOUBLE");

    let profile = engine.profile("sales").unwrap();
    assert_eq!(total_rows(&profile), 3, "one SUMMARIZE row per column");
}

#[test]
fn relative_paths_resolve_against_base_dir() {
    let dir = tempfile::tempdir().unwrap();
    write_sample(dir.path());
    let engine = Engine::new().unwrap();
    let source = Source {
        name: "sales".into(),
        kind: SourceKind::File {
            path: "sample.csv".into(),
            format: dv_core::FileFormat::Csv,
        },
    };
    engine.register_source(&source, dir.path()).unwrap();
    let batches = engine.query("SELECT count(*) FROM sales").unwrap();
    let count = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(count.value(0), 4);
}

#[test]
fn missing_file_is_io_error() {
    let engine = Engine::new().unwrap();
    let err = engine
        .register_source(
            &csv_source("nope", Path::new("no-such-file.csv")),
            Path::new(""),
        )
        .unwrap_err();
    assert!(matches!(err, EngineError::Io { .. }));
}

/// Runs a query that returns one scalar and reads it as i64 (cast, since `sum` yields HUGEINT).
fn first_count(engine: &Engine, sql: &str) -> i64 {
    let batches = engine
        .query(&format!("SELECT CAST(({sql}) AS BIGINT)"))
        .unwrap();
    batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap()
        .value(0)
}

/// Creates a SQLite file with two tables through DuckDB's sqlite extension.
/// Needs the extension, which DuckDB downloads on first use.
fn sqlite_fixture(path: &Path) {
    // Tests run in parallel, and several installing the extension at once race on the same file
    // in ~/.duckdb (a rename that fails with "Access is denied" on Windows). Install one at a time.
    static INSTALL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let maker = duckdb::Connection::open_in_memory().unwrap();
    {
        let _one_at_a_time = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
        maker.execute_batch("INSTALL sqlite; LOAD sqlite;").expect(
            "creating the SQLite fixture needs the DuckDB sqlite extension (network on first use)",
        );
    }
    maker
        .execute_batch(&format!(
            "ATTACH {} AS fx (TYPE sqlite); \
             CREATE TABLE fx.orders (id INTEGER, region VARCHAR, units INTEGER); \
             INSERT INTO fx.orders VALUES (1, 'North', 3), (2, 'South', 5), (3, 'North', 2), (4, 'West', 7); \
             CREATE TABLE fx.customers (name VARCHAR); \
             INSERT INTO fx.customers VALUES ('Ann'), ('Bo'); \
             DETACH fx;",
            sql::quote_literal(&path.to_string_lossy())
        ))
        .expect("creating the SQLite fixture needs the DuckDB sqlite extension (network on first use)");
}

#[test]
fn sqlite_tables_are_exposed_as_read_only_views() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.sqlite");
    sqlite_fixture(&path);

    let engine = Engine::new().unwrap();
    engine
        .register_database("wh", dv_core::DbKind::Sqlite, &path.to_string_lossy())
        .unwrap();

    assert_eq!(
        first_count(&engine, r#"SELECT count(*) FROM "wh"."orders""#),
        4
    );
    assert_eq!(
        first_count(&engine, r#"SELECT count(*) FROM "wh"."customers""#),
        2
    );
    assert_eq!(
        first_count(
            &engine,
            r#"SELECT sum(units) FROM "wh"."orders" WHERE region = 'North'"#
        ),
        5
    );

    // The attachment is READ_ONLY, so writes through the views must fail.
    assert!(
        engine
            .execute(r#"INSERT INTO "wh"."orders" VALUES (9, 'East', 1)"#)
            .is_err()
    );
}

#[test]
fn database_source_with_sql_dataset_queries_a_table() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.sqlite");
    sqlite_fixture(&path);

    let engine = Engine::new().unwrap();
    engine
        .register_database("wh", dv_core::DbKind::Sqlite, &path.to_string_lossy())
        .unwrap();
    engine
        .register_dataset(&dv_core::Dataset {
            name: "big_orders".into(),
            source: "wh".into(),
            sql: Some(r#"SELECT * FROM "wh"."orders" WHERE units >= 5"#.into()),
            joins: Vec::new(),
            calculated_fields: vec![],
        })
        .unwrap();
    assert_eq!(
        total_rows(&engine.query(r#"SELECT * FROM "big_orders""#).unwrap()),
        2
    );
}

#[test]
fn database_dataset_without_sql_is_rejected_with_a_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fixture.sqlite");
    sqlite_fixture(&path);

    let engine = Engine::new().unwrap();
    engine
        .register_database("wh", dv_core::DbKind::Sqlite, &path.to_string_lossy())
        .unwrap();
    let err = engine
        .register_dataset(&dv_core::Dataset {
            name: "all".into(),
            source: "wh".into(),
            sql: None,
            joins: Vec::new(),
            calculated_fields: vec![],
        })
        .unwrap_err();
    assert!(matches!(err, EngineError::InvalidSpec(_)));
}

#[test]
fn database_source_reads_connection_from_environment() {
    // The variable name is unlikely to be set on any machine.
    let source = Source {
        name: "wh".into(),
        kind: SourceKind::Database {
            kind: dv_core::DbKind::Postgres,
            connection: "dv-engine-test-unset".into(),
        },
    };
    let engine = Engine::new().unwrap();
    match engine.register_source(&source, Path::new("")).unwrap_err() {
        EngineError::MissingConnection {
            connection,
            env_var,
        } => {
            assert_eq!(connection, "dv-engine-test-unset");
            assert_eq!(env_var, "DAVILIZE_CONN_DV_ENGINE_TEST_UNSET");
        }
        other => panic!("expected MissingConnection, got {other:?}"),
    }
}

#[test]
fn connection_references_are_validated_before_lookup() {
    assert_eq!(
        crate::connection_env_var("warehouse-prod").unwrap(),
        "DAVILIZE_CONN_WAREHOUSE_PROD"
    );
    for bad in ["", "a\"; DROP TABLE x; --", "a/b", "a b", "a.b"] {
        assert!(
            matches!(
                crate::connection_env_var(bad),
                Err(EngineError::InvalidSpec(_))
            ),
            "{bad:?} should be rejected"
        );
    }
}

#[test]
fn attach_errors_do_not_leak_the_target() {
    let engine = Engine::new().unwrap();
    let secret = "/definitely/missing/dir/secret-token-123.sqlite";
    let err = engine
        .register_database("wh", dv_core::DbKind::Sqlite, secret)
        .unwrap_err();
    assert!(!err.to_string().contains("secret-token-123"), "{err}");
}

/// Runs against a real Postgres when `DAVILIZE_TEST_POSTGRES` holds a connection
/// string for a disposable database. Otherwise the test is skipped.
#[test]
fn postgres_source_reads_tables_read_only() {
    let Ok(target) = std::env::var("DAVILIZE_TEST_POSTGRES") else {
        eprintln!("skipped: DAVILIZE_TEST_POSTGRES is not set");
        return;
    };

    // Create the fixture table with a separate writable attachment.
    let maker = duckdb::Connection::open_in_memory().unwrap();
    maker
        .execute_batch(&format!(
            "INSTALL postgres; LOAD postgres; \
             ATTACH {} AS pgw (TYPE postgres); \
             CREATE OR REPLACE TABLE pgw.public.davilize_smoke AS SELECT * FROM (VALUES (1, 'a'), (2, 'b'), (3, 'c')) t(id, label); \
             DETACH pgw;",
            sql::quote_literal(&target)
        ))
        .expect("fixture setup on the test Postgres failed");

    let engine = Engine::new().unwrap();
    engine
        .register_database("pg", dv_core::DbKind::Postgres, &target)
        .unwrap();
    assert_eq!(
        first_count(&engine, r#"SELECT count(*) FROM "pg"."davilize_smoke""#),
        3
    );
    assert!(
        engine
            .execute(r#"INSERT INTO "pg"."davilize_smoke" VALUES (4, 'd')"#)
            .is_err()
    );
}

/// Writes an Arrow IPC file with one batch per inner vector.
fn write_arrow_file(path: &Path, batches: &[Vec<(i32, &str)>]) {
    use std::sync::Arc;

    use duckdb::arrow::array::{ArrayRef, Int32Array, StringArray};
    use duckdb::arrow::datatypes::{DataType, Field, Schema};
    use duckdb::arrow::ipc::writer::FileWriter;

    let schema = Arc::new(Schema::new(vec![
        Field::new("units", DataType::Int32, false),
        Field::new("region", DataType::Utf8, true),
    ]));
    let file = fs::File::create(path).unwrap();
    let mut writer = FileWriter::try_new(file, &schema).unwrap();
    for rows in batches {
        let units: Vec<i32> = rows.iter().map(|(u, _)| *u).collect();
        let regions: Vec<&str> = rows.iter().map(|(_, r)| *r).collect();
        let columns: Vec<ArrayRef> = vec![
            Arc::new(Int32Array::from(units)),
            Arc::new(StringArray::from(regions)),
        ];
        writer
            .write(
                &duckdb::arrow::record_batch::RecordBatch::try_new(schema.clone(), columns)
                    .unwrap(),
            )
            .unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn arrow_ipc_file_is_queryable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sales.arrow");
    // Two batches, so both the initial CREATE and the per-batch INSERT run.
    write_arrow_file(
        &path,
        &[
            vec![(3, "North"), (5, "South")],
            vec![(2, "North"), (7, "West"), (1, "East")],
        ],
    );

    let engine = Engine::new().unwrap();
    engine
        .register_source(
            &Source {
                name: "sales_arrow".into(),
                kind: SourceKind::File {
                    path: "sales.arrow".into(),
                    format: dv_core::FileFormat::Arrow,
                },
            },
            dir.path(),
        )
        .unwrap();

    assert_eq!(
        first_count(&engine, r#"SELECT count(*) FROM "sales_arrow""#),
        5
    );
    assert_eq!(
        first_count(&engine, r#"SELECT sum(units) FROM "sales_arrow""#),
        18
    );
    assert_eq!(
        first_count(
            &engine,
            r#"SELECT count(*) FROM "sales_arrow" WHERE region = 'North'"#
        ),
        2
    );
    let schema = engine.schema("sales_arrow").unwrap();
    assert_eq!(schema[0].name, "units");
    assert_eq!(schema[0].data_type, "INTEGER");
    assert_eq!(schema[1].data_type, "VARCHAR");
}

#[test]
fn feather_extension_is_inferred_and_empty_files_work() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.feather");
    write_arrow_file(&path, &[]);

    let engine = Engine::new().unwrap();
    let format = engine.register_file("empty", &path).unwrap();
    assert_eq!(format, dv_core::FileFormat::Arrow);
    assert_eq!(first_count(&engine, r#"SELECT count(*) FROM "empty""#), 0);
    assert_eq!(engine.schema("empty").unwrap().len(), 2);
}

#[test]
fn invalid_arrow_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bogus.arrow");
    fs::write(&path, "not arrow").unwrap();
    let engine = Engine::new().unwrap();
    let err = engine.register_file("bogus", &path).unwrap_err();
    assert!(matches!(err, EngineError::Unsupported(_)), "{err}");
}

#[test]
fn infers_formats_from_extension() {
    assert_eq!(
        infer_format(Path::new("a.CSV")).unwrap(),
        dv_core::FileFormat::Csv
    );
    assert_eq!(
        infer_format(Path::new("a.parquet")).unwrap(),
        dv_core::FileFormat::Parquet
    );
    assert_eq!(
        infer_format(Path::new("a.ndjson")).unwrap(),
        dv_core::FileFormat::Json
    );
    assert!(infer_format(Path::new("a.xlsx")).is_err());
}

#[test]
fn visual_runs_end_to_end_on_csv() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_sample(dir.path());
    let engine = Engine::new().unwrap();
    engine
        .register_source(&csv_source("sales", &path), Path::new(""))
        .unwrap();

    let visual = Visual {
        id: "by_region".into(),
        title: "Units by region".into(),
        dataset: "sales".into(),
        chart: ChartType::Bar,
        encodings: Encodings::default(),
        dimensions: vec!["region".into()],
        measures: vec![Measure {
            field: "units".into(),
            agg: Aggregate::Sum,
            alias: Some("total_units".into()),
        }],
        filters: vec![],
        sort: None,
        limit: None,
    };
    let batches = engine.run_visual(&visual).unwrap();
    assert_eq!(total_rows(&batches), 3);
}

#[test]
fn dataset_sql_becomes_a_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_sample(dir.path());
    let mut report = Report::new("r");
    report.sources.push(csv_source("raw", &path));
    report.datasets.push(dv_core::Dataset {
        name: "big_orders".into(),
        source: "raw".into(),
        sql: Some("SELECT * FROM raw WHERE units >= 5".into()),
        joins: Vec::new(),
        calculated_fields: vec![],
    });
    let engine = Engine::new().unwrap();
    engine.register_report(&report, Path::new("")).unwrap();
    let batches = engine.query(r#"SELECT * FROM "big_orders""#).unwrap();
    assert_eq!(total_rows(&batches), 2);
}

#[test]
fn export_csv_and_parquet_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_sample(dir.path());
    let engine = Engine::new().unwrap();
    engine
        .register_source(&csv_source("sales", &path), Path::new(""))
        .unwrap();

    let csv_out = dir.path().join("out.csv");
    let rows = engine
        .export(
            "SELECT region, units FROM sales",
            &csv_out,
            OutputFormat::Csv,
        )
        .unwrap();
    assert_eq!(rows, 4);
    let text = fs::read_to_string(&csv_out).unwrap();
    assert!(text.starts_with("region,units"));

    let parquet_out = dir.path().join("out.parquet");
    engine
        .export(
            "SELECT region, units FROM sales",
            &parquet_out,
            OutputFormat::Parquet,
        )
        .unwrap();
    let reader = Engine::new().unwrap();
    reader.register_file("back", &parquet_out).unwrap();
    assert_eq!(total_rows(&reader.query("SELECT * FROM back").unwrap()), 4);
}

#[test]
fn sql_errors_surface_as_duckdb_errors() {
    let engine = Engine::new().unwrap();
    let err = engine.query("SELECT * FROM does_not_exist").unwrap_err();
    assert!(matches!(err, EngineError::Duckdb(_)));
}

#[test]
fn awkward_csv_columns_are_read_as_numbers_and_dates() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("awkward.csv");
    let mut csv = String::from("id,zip,price,pct,joined,qty,note\n");
    for i in 1..=30 {
        let qty = if i % 5 == 0 {
            "n/a".to_string()
        } else {
            i.to_string()
        };
        csv.push_str(&format!(
            "{i},{:05},\"${},{:03}.50\",{}%,Jan {} 2025,{qty},ok\n",
            2000 + i,
            i,
            i,
            i % 100,
            i % 27 + 1,
        ));
    }
    fs::write(&path, csv).unwrap();

    let engine = Engine::new().unwrap();
    engine
        .register_source(&csv_source("awkward", &path), Path::new(""))
        .unwrap();
    let types: Vec<(String, String)> = engine
        .schema("awkward")
        .unwrap()
        .into_iter()
        .map(|c| (c.name, c.data_type))
        .collect();
    let type_of = |name: &str| {
        types
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t.as_str())
            .unwrap()
    };
    assert_eq!(type_of("id"), "BIGINT");
    assert_eq!(
        type_of("zip"),
        "VARCHAR",
        "codes with leading zeros stay text"
    );
    assert_eq!(type_of("price"), "DOUBLE");
    assert_eq!(type_of("pct"), "BIGINT");
    assert_eq!(type_of("joined"), "DATE");
    assert_eq!(
        type_of("qty"),
        "BIGINT",
        "n/a among numbers is read as empty"
    );
    assert_eq!(type_of("note"), "VARCHAR");

    // The values themselves are right: "$1,001.50" is 1001.5, and "Jan 2 2025" is a date.
    assert_eq!(
        first_count(
            &engine,
            r#"SELECT count(*) FROM "awkward" WHERE price = 1001.5"#
        ),
        1
    );
    assert_eq!(
        first_count(
            &engine,
            r#"SELECT count(*) FROM "awkward" WHERE qty IS NULL"#
        ),
        6
    );
    assert_eq!(
        first_count(
            &engine,
            r#"SELECT count(*) FROM "awkward" WHERE joined = DATE '2025-01-02'"#
        ),
        2
    );
}
