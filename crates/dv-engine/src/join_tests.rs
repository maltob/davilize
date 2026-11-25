use std::fs;
use std::path::Path;

use duckdb::arrow::array::Int64Array;
use dv_core::{CalculatedField, Dataset, FileFormat, Join, JoinKey, JoinKind, Source, SourceKind};

use crate::{Engine, EngineError};

const SALES: &str = "region,units,revenue\nNorth,3,30.0\nSouth,5,55.5\nNorth,2,20.0\nWest,7,70.0\n";
const REGIONS: &str = "region,manager\nNorth,Ann\nSouth,Bo\nEast,Cy\n";
/// `Region` is spelled differently from `sales.region` on purpose, and `units` collides.
const TARGETS: &str = "Region,units,target\nNorth,3,30\nNorth,2,20\nSouth,9,1\n";

/// An engine with the `sales`, `regions` and `targets` CSV files registered as sources.
fn engine_with_sources(dir: &Path) -> Engine {
    let engine = Engine::new().unwrap();
    for (name, text) in [("sales", SALES), ("regions", REGIONS), ("targets", TARGETS)] {
        let path = dir.join(format!("{name}.csv"));
        fs::write(&path, text).unwrap();
        let source = Source {
            name: name.into(),
            kind: SourceKind::File {
                path,
                format: FileFormat::Csv,
            },
        };
        engine.register_source(&source, Path::new("")).unwrap();
    }
    engine
}

fn join(with: &str, kind: JoinKind, keys: &[(&str, &str)]) -> Join {
    Join {
        with: with.into(),
        kind,
        on: keys
            .iter()
            .map(|(left, right)| JoinKey {
                left: (*left).into(),
                right: (*right).into(),
            })
            .collect(),
    }
}

fn dataset(name: &str, source: &str, joins: Vec<Join>) -> Dataset {
    Dataset {
        name: name.into(),
        source: source.into(),
        sql: None,
        joins,
        calculated_fields: vec![],
    }
}

/// Runs a query that returns one count and reads it as i64.
fn count(engine: &Engine, sql: &str) -> i64 {
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

fn columns(engine: &Engine, relation: &str) -> Vec<String> {
    engine
        .schema(relation)
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect()
}

/// Registers `dataset` and returns the error it must fail with.
fn invalid_spec(engine: &Engine, dataset: &Dataset) -> String {
    match engine.register_dataset(dataset).unwrap_err() {
        EngineError::InvalidSpec(message) => message,
        other => panic!("expected InvalidSpec, got {other:?}"),
    }
}

#[test]
fn inner_join_keeps_only_matching_rows() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join("regions", JoinKind::Inner, &[("region", "region")])],
        ))
        .unwrap();
    // North (twice) and South match; West and East do not.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 3);
    assert_eq!(
        columns(&engine, "j"),
        ["region", "units", "revenue", "regions_region", "manager"]
    );
}

#[test]
fn default_join_kind_is_inner() {
    let text = r#"{"with": "regions", "on": [{"left": "region", "right": "region"}]}"#;
    let join: Join = serde_json::from_str(text).unwrap();
    assert_eq!(join.kind, JoinKind::Inner);
}

#[test]
fn left_join_keeps_unmatched_left_rows_with_nulls() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join("regions", JoinKind::Left, &[("region", "region")])],
        ))
        .unwrap();
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 4);
    assert_eq!(
        count(
            &engine,
            r#"SELECT count(*) FROM "j" WHERE region = 'West' AND manager IS NULL"#
        ),
        1
    );
}

#[test]
fn right_join_keeps_unmatched_right_rows_with_nulls() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join("regions", JoinKind::Right, &[("region", "region")])],
        ))
        .unwrap();
    // Three matches plus East, which has no sales row.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 4);
    assert_eq!(
        count(
            &engine,
            r#"SELECT count(*) FROM "j" WHERE manager = 'Cy' AND region IS NULL AND units IS NULL"#
        ),
        1
    );
}

#[test]
fn full_join_keeps_unmatched_rows_from_both_sides() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join("regions", JoinKind::Full, &[("region", "region")])],
        ))
        .unwrap();
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 5);
    assert_eq!(
        count(&engine, r#"SELECT count(*) FROM "j" WHERE manager IS NULL"#),
        1
    );
    assert_eq!(
        count(&engine, r#"SELECT count(*) FROM "j" WHERE region IS NULL"#),
        1
    );
}

#[test]
fn colliding_column_names_are_renamed_case_insensitively() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join("targets", JoinKind::Inner, &[("region", "Region")])],
        ))
        .unwrap();
    // "Region" collides with "region" and "units" with "units"; "target" is free.
    assert_eq!(
        columns(&engine, "j"),
        [
            "region",
            "units",
            "revenue",
            "targets_Region",
            "targets_units",
            "target"
        ]
    );
    // North rows (two) match both North targets, South matches its one target.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 5);
}

#[test]
fn repeated_join_numbers_colliding_names() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "twice",
            "sales",
            vec![
                join("targets", JoinKind::Inner, &[("region", "Region")]),
                join("targets", JoinKind::Inner, &[("region", "Region")]),
            ],
        ))
        .unwrap();
    assert_eq!(
        columns(&engine, "twice"),
        [
            "region",
            "units",
            "revenue",
            "targets_Region",
            "targets_units",
            "target",
            "targets_Region_2",
            "targets_units_2",
            "targets_target",
        ]
    );
    // Five rows after the first join; North rows (four) match two targets each, South one.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "twice""#), 9);
}

#[test]
fn multi_key_join_requires_every_key() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    engine
        .register_dataset(&dataset(
            "j",
            "sales",
            vec![join(
                "targets",
                JoinKind::Inner,
                &[("region", "Region"), ("units", "units")],
            )],
        ))
        .unwrap();
    // Only North/3 and North/2 match on both keys; South 5 has no target with 5 units.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "j""#), 2);
}

#[test]
fn join_can_follow_an_earlier_joined_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    let report = dv_core::Report {
        version: dv_core::REPORT_VERSION,
        name: "r".into(),
        sources: vec![],
        datasets: vec![
            dataset(
                "with_managers",
                "sales",
                vec![join("regions", JoinKind::Inner, &[("region", "region")])],
            ),
            dataset(
                "managed",
                "with_managers",
                vec![join("targets", JoinKind::Inner, &[("region", "Region")])],
            ),
        ],
        visuals: vec![],
        parameters: vec![],
        dashboards: vec![],
        schedules: vec![],
    };
    for dataset in &report.datasets {
        engine.register_dataset(dataset).unwrap();
    }
    // Three managed sales rows: North rows match two targets each, South matches one.
    assert_eq!(count(&engine, r#"SELECT count(*) FROM "managed""#), 5);
    let names = columns(&engine, "managed");
    assert!(names.iter().any(|c| c == "manager"), "{names:?}");
    assert!(names.iter().any(|c| c == "target"), "{names:?}");
}

#[test]
fn joins_work_with_calculated_fields() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    let mut joined = dataset(
        "j",
        "sales",
        vec![join("regions", JoinKind::Left, &[("region", "region")])],
    );
    joined.calculated_fields = vec![CalculatedField {
        name: "per_unit".into(),
        expression: r#""revenue" / "units""#.into(),
        description: None,
    }];
    engine.register_dataset(&joined).unwrap();
    assert!(columns(&engine, "j").iter().any(|c| c == "per_unit"));
    // West has no manager, but its calculated field is still computed: 70 / 7.
    assert_eq!(
        count(
            &engine,
            r#"SELECT count(*) FROM "j" WHERE region = 'West' AND manager IS NULL AND per_unit = 10"#
        ),
        1
    );
}

#[test]
fn unknown_key_column_is_a_clear_error() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());

    let left = dataset(
        "j",
        "sales",
        vec![join("regions", JoinKind::Inner, &[("nope", "region")])],
    );
    let message = invalid_spec(&engine, &left);
    assert!(
        message.contains("join on dataset 'j': column 'nope' not found in 'sales'"),
        "{message}"
    );

    let right = dataset(
        "j",
        "sales",
        vec![join("regions", JoinKind::Inner, &[("region", "nope")])],
    );
    let message = invalid_spec(&engine, &right);
    assert!(
        message.contains("column 'nope' not found in 'regions'"),
        "{message}"
    );
}

#[test]
fn unknown_with_and_source_are_clear_errors() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());

    let missing_with = dataset(
        "j",
        "sales",
        vec![join("missing", JoinKind::Inner, &[("region", "region")])],
    );
    let message = invalid_spec(&engine, &missing_with);
    assert!(
        message.contains("joins unknown source 'missing'"),
        "{message}"
    );

    let missing_source = dataset(
        "j",
        "missing",
        vec![join("regions", JoinKind::Inner, &[("region", "region")])],
    );
    let message = invalid_spec(&engine, &missing_source);
    assert!(
        message.contains("references unknown source 'missing'"),
        "{message}"
    );
}

#[test]
fn joins_cannot_be_combined_with_sql() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine_with_sources(dir.path());
    let mut both = dataset(
        "j",
        "sales",
        vec![join("regions", JoinKind::Inner, &[("region", "region")])],
    );
    both.sql = Some("SELECT * FROM sales".into());
    let message = invalid_spec(&engine, &both);
    assert!(message.contains("both sql and joins"), "{message}");
}
