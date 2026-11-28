use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use dv_core::{
    Aggregate, CalculatedField, ChartType, Dashboard, DashboardItem, Encodings, Filter, FilterOp,
    FilterScope, Measure, Parameter, Report, Visual,
};
use serde_json::json;

use super::*;

const CSV: &str = "order_id,region,product,units,revenue,when\n\
1,North,Widget,4,50.0,2026-01-03\n\
2,South,Gadget,2,60.0,2026-01-05\n\
3,North,Gizmo,1,99.0,2026-01-07\n\
4,West,Widget,10,125.0,2026-01-09\n\
5,,Gadget,3,90.0,2026-01-12\n";

fn write_csv(dir: &Path) -> PathBuf {
    let path = dir.join("sales.csv");
    fs::write(&path, CSV).unwrap();
    path
}

/// Adds `sales.csv` to a fresh engine and returns the engine with the added dataset.
fn engine_with_sales(dir: &Path) -> (Engine, AddedDataset) {
    let engine = Engine::new().unwrap();
    let added = add_data_files(&engine, &[], &[write_csv(dir)]).unwrap();
    (engine, added.into_iter().next().unwrap())
}

fn revenue_by_region() -> Visual {
    Visual {
        id: "revenue_by_region".into(),
        title: "Revenue by region".into(),
        dataset: "sales".into(),
        chart: ChartType::Bar,
        encodings: Encodings {
            x: Some("region".into()),
            y: Some("sum_revenue".into()),
            color: None,
            size: None,
        },
        dimensions: vec!["region".into()],
        measures: vec![Measure {
            field: "revenue".into(),
            agg: Aggregate::Sum,
            alias: None,
        }],
        filters: vec![],
        sort: None,
        limit: None,
    }
}

/// A report holding the added dataset and one visual.
fn report_with(added: &AddedDataset, visual: Visual) -> Report {
    let mut report = Report::new("Sample sales");
    report.sources.push(added.source.clone());
    report.datasets.push(added.dataset.clone());
    report.visuals.push(visual);
    report
}

#[test]
fn column_families_and_default_roles() {
    assert_eq!(family("INTEGER"), "number");
    assert_eq!(family("DECIMAL(10,2)"), "number");
    assert_eq!(family("VARCHAR"), "text");
    assert_eq!(family("DATE"), "date");
    assert_eq!(family("TIMESTAMP WITH TIME ZONE"), "date");
    assert_eq!(family("BOOLEAN"), "bool");
    assert_eq!(family("STRUCT(a INTEGER)"), "other");

    let columns = column_views(vec![
        ColumnInfo {
            name: "units".into(),
            data_type: "BIGINT".into(),
            nullable: true,
            calculated: false,
        },
        ColumnInfo {
            name: "region".into(),
            data_type: "VARCHAR".into(),
            nullable: true,
            calculated: false,
        },
    ]);
    assert_eq!(columns[0].role, "measure");
    assert_eq!(columns[1].role, "dimension");
}

#[test]
fn identifier_columns_start_as_dimensions() {
    for name in [
        "id",
        "Id",
        "order_id",
        "ORDER_ID",
        "customerId",
        "product_key",
    ] {
        assert!(is_identifier(name, "BIGINT"), "{name}");
    }
    // Not identifiers: other names, non-whole numbers, and words that merely end in "id".
    for (name, ty) in [
        ("units", "BIGINT"),
        ("paid", "INTEGER"),
        ("valid", "INTEGER"),
        ("order_id", "DOUBLE"),
        ("order_id", "VARCHAR"),
        ("grid", "INTEGER"),
    ] {
        assert!(!is_identifier(name, ty), "{name} {ty}");
    }
}

#[test]
fn relation_names_are_sanitized_and_unique() {
    assert_eq!(relation_base("my sales-2026.v1"), "my_sales_2026_v1");
    assert_eq!(relation_base("---"), "data");

    let mut taken = HashSet::new();
    assert_eq!(unique_relation("sales", &taken), "sales");
    taken.insert("sales".to_string());
    assert_eq!(unique_relation("sales", &taken), "sales_2");

    let mut taken = HashSet::new();
    taken.insert("sales_source".to_string());
    assert_eq!(unique_relation("sales", &taken), "sales_2");
}

#[test]
fn output_stems_are_file_safe() {
    assert_eq!(output_stem("Sample sales"), "Sample_sales");
    assert_eq!(output_stem("../../etc"), "etc");
    assert_eq!(output_stem("  "), "report");
}

#[test]
fn export_format_names() {
    assert_eq!(ExportFormat::parse("xlsx").unwrap(), ExportFormat::Xlsx);
    let err = ExportFormat::parse("gif").unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn add_files_registers_source_dataset_and_schema() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    assert_eq!(added.dataset.name, "sales");
    assert_eq!(added.dataset.source, "sales_source");
    assert!(added.dataset.sql.is_none());
    let units = added.columns.iter().find(|c| c.name == "units").unwrap();
    assert_eq!(units.role, "measure");
    let region = added.columns.iter().find(|c| c.name == "region").unwrap();
    assert_eq!(region.role, "dimension");
    assert_eq!(region.family, "text");

    // The engine answers queries over the registered dataset.
    let table = preview(&engine, "sales", 10).unwrap();
    assert_eq!(table.rows.len(), 5);
}

#[test]
fn adding_the_same_file_twice_gets_a_new_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_csv(dir.path());
    let engine = Engine::new().unwrap();
    let first = add_data_files(&engine, &[], std::slice::from_ref(&path)).unwrap();
    let taken = vec![first[0].dataset.name.clone(), first[0].source.name.clone()];
    let second = add_data_files(&engine, &taken, &[path]).unwrap();
    assert_eq!(second[0].dataset.name, "sales_2");
    assert_eq!(second[0].source.name, "sales_2_source");
}

#[test]
fn missing_file_is_an_io_error() {
    let engine = Engine::new().unwrap();
    let err = add_data_files(&engine, &[], &[PathBuf::from("no-such-file.csv")]).unwrap_err();
    assert_eq!(err.kind, "io");
}

#[test]
fn unsupported_extension_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    fs::write(&path, "hello").unwrap();
    let engine = Engine::new().unwrap();
    let err = add_data_files(&engine, &[], &[path]).unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn preview_is_capped_and_keeps_nulls() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());

    let two = preview(&engine, "sales", 2).unwrap();
    assert_eq!(two.rows.len(), 2);
    assert_eq!(two.rows[0][1].as_deref(), Some("North"));
    assert_eq!(two.columns[0].name, "order_id");

    let clamped = preview(&engine, "sales", 0).unwrap();
    assert_eq!(clamped.rows.len(), 1);
    let all = preview(&engine, "sales", u32::MAX).unwrap();
    assert_eq!(all.rows.len(), 5);
    assert_eq!(all.rows[4][1], None, "empty CSV field is SQL NULL");
}

#[test]
fn profile_has_one_row_per_column() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let table = profile(&engine, "sales").unwrap();
    assert_eq!(table.rows.len(), 6);
    assert_eq!(table.columns[0].name, "column_name");
}

#[test]
fn render_visual_builds_spec_from_engine_result() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let rendered = render_visual(&engine, &revenue_by_region(), &[], &BTreeMap::new()).unwrap();
    assert_eq!(rendered.spec["mark"]["type"], "bar");
    assert!(rendered.sql.contains("GROUP BY"));
    // North, South, West and the NULL region.
    assert_eq!(rendered.row_count, 4);
    assert!(rendered.spec["data"]["values"].is_array());
}

#[test]
fn render_visual_reports_unknown_fields_as_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let mut visual = revenue_by_region();
    visual.encodings.x = Some("nope".into());
    let err = render_visual(&engine, &visual, &[], &BTreeMap::new()).unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn render_visual_reports_missing_dataset_as_query_error() {
    let engine = Engine::new().unwrap();
    let err = render_visual(&engine, &revenue_by_region(), &[], &BTreeMap::new()).unwrap_err();
    assert_eq!(err.kind, "query");
}

#[test]
fn interrupting_a_running_query_reports_cancelled() {
    let engine = Engine::new().unwrap();
    let handle = engine.interrupt_handle();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        handle.interrupt();
    });
    let result = engine.query("SELECT sum(i) FROM range(100000000000) t(i)");
    stopper.join().unwrap();
    let err = ApiError::from(result.unwrap_err());
    assert_eq!(err.kind, "cancelled");
}

#[test]
fn export_csv_writes_one_file_per_visual() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let report = report_with(&added, revenue_by_region());
    let out = dir.path().join("out");
    let summary =
        export_report(&engine, &report, ExportFormat::Csv, &out, &BTreeMap::new()).unwrap();
    assert_eq!(summary.files.len(), 1);
    let text = fs::read_to_string(out.join("revenue_by_region.csv")).unwrap();
    assert!(text.starts_with("region,sum_revenue"), "{text}");
}

#[test]
fn export_html_and_xlsx_write_report_files() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let report = report_with(&added, revenue_by_region());
    let out = dir.path().join("out");

    let html = export_report(&engine, &report, ExportFormat::Html, &out, &BTreeMap::new()).unwrap();
    assert_eq!(html.files.len(), 1);
    let page = fs::read_to_string(out.join("Sample_sales.html")).unwrap();
    assert!(page.contains("<h2>Revenue by region</h2>"));
    assert!(page.contains("vega-embed"));

    export_report(&engine, &report, ExportFormat::Xlsx, &out, &BTreeMap::new()).unwrap();
    let book = fs::read(out.join("Sample_sales.xlsx")).unwrap();
    assert!(book.starts_with(b"PK"), "xlsx is a zip container");
}

#[test]
fn export_png_writes_bar_and_table_charts() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut table = revenue_by_region();
    table.id = "as_table".into();
    table.chart = ChartType::Table;
    let report = report_with(&added, revenue_by_region());
    let out = dir.path().join("png");

    let summary =
        export_report(&engine, &report, ExportFormat::Png, &out, &BTreeMap::new()).unwrap();
    assert!(summary.warnings.is_empty());
    let image = fs::read(out.join("revenue_by_region.png")).unwrap();
    assert!(image.starts_with(&[0x89, b'P', b'N', b'G']));

    let mut report = report_with(&added, table);
    report.visuals[0].id = "as_table".into();
    let summary =
        export_report(&engine, &report, ExportFormat::Png, &out, &BTreeMap::new()).unwrap();
    assert!(summary.warnings.is_empty(), "{:?}", summary.warnings);
    let image = fs::read(out.join("as_table.png")).unwrap();
    assert!(image.starts_with(&[0x89, b'P', b'N', b'G']));
}

#[test]
fn export_rejects_ids_that_are_not_safe_file_names() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut visual = revenue_by_region();
    visual.id = "../escape".into();
    let report = report_with(&added, visual);
    let err = export_report(
        &engine,
        &report,
        ExportFormat::Csv,
        &dir.path().join("out"),
        &BTreeMap::new(),
    )
    .unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(!dir.path().join("escape.csv").exists());
}

#[test]
fn save_stores_source_paths_relative_to_the_report_and_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(dir.path());
    let report = report_with(&added, revenue_by_region());
    let path = dir.path().join("sample.dvz");

    let stored = save_report(&path, &report).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"path\": \"sales.csv\""), "{text}");
    assert!(!text.contains(&dir.path().display().to_string().replace('\\', "\\\\")));
    match &stored.sources[0].kind {
        SourceKind::File { path, .. } => assert_eq!(path, Path::new("sales.csv")),
        other => panic!("unexpected source kind {other:?}"),
    }

    let (engine, opened) = open_report(&path).unwrap();
    assert_eq!(opened.report.name, "Sample sales");
    assert_eq!(opened.datasets[0].name, "sales");
    let table = preview(&engine, "sales", 3).unwrap();
    assert_eq!(table.rows.len(), 3);
    let rendered =
        render_visual(&engine, &opened.report.visuals[0], &[], &BTreeMap::new()).unwrap();
    assert_eq!(rendered.row_count, 4);
}

#[test]
fn save_as_into_another_folder_keeps_sources_reachable() {
    let data_dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(data_dir.path());
    let report = report_with(&added, revenue_by_region());
    let first = data_dir.path().join("a.dvz");
    save_report(&first, &report).unwrap();

    // Reopen (sources stored relative), then "Save as" into an unrelated folder.
    let (_engine, opened) = open_report(&first).unwrap();
    let other_dir = tempfile::tempdir().unwrap();
    let second = other_dir.path().join("b.dvz");
    save_report(&second, &opened.report).unwrap();

    let (engine, reopened) = open_report(&second).unwrap();
    let rendered =
        render_visual(&engine, &reopened.report.visuals[0], &[], &BTreeMap::new()).unwrap();
    assert_eq!(rendered.row_count, 4);
}

#[test]
fn save_rejects_invalid_reports() {
    let dir = tempfile::tempdir().unwrap();
    let mut report = Report::new("bad");
    report.version = 2;
    let err = save_report(&dir.path().join("bad.dvz"), &report).unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(!dir.path().join("bad.dvz").exists());
}

#[test]
fn sample_report_writes_files_and_every_chart_renders() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join(SAMPLE_FOLDER);
    let path = write_sample_report(&folder).unwrap();
    assert!(folder.is_absolute());
    assert_eq!(path, folder.join("sample.dvz"));
    assert_eq!(
        fs::read_to_string(folder.join("sample.csv")).unwrap(),
        SAMPLE_CSV
    );
    // A second write replaces the earlier copies.
    assert_eq!(write_sample_report(&folder).unwrap(), path);

    // Open it the way the app opens a report, then render every chart and the dashboard.
    let (engine, opened) = open_report(&path).unwrap();
    assert_eq!(opened.report.visuals.len(), 2);
    assert_eq!(opened.report.dashboards.len(), 1);
    for visual in &opened.report.visuals {
        let rendered = render_visual(&engine, visual, &[], &BTreeMap::new()).unwrap();
        assert!(rendered.row_count > 0, "{} has no rows", visual.id);
    }
    let dashboard = &opened.report.dashboards[0];
    for item in &dashboard.items {
        let visual = opened
            .report
            .visuals
            .iter()
            .find(|v| v.id == item.visual)
            .expect("dashboard item names a visual");
        let rendered =
            render_dashboard_visual(&engine, visual, &dashboard.filters, &[], &BTreeMap::new())
                .unwrap();
        assert!(
            rendered.row_count > 0,
            "{} has no rows on the dashboard",
            visual.id
        );
    }
    // The stored report passes the same validation as a save.
    save_report(&dir.path().join("copy.dvz"), &opened.report).unwrap();
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// revenue_by_region, keeping rows where `units >= {param: min_units}`.
fn revenue_from_min_units() -> Visual {
    let mut visual = revenue_by_region();
    visual.filters.push(Filter {
        field: "units".into(),
        op: FilterOp::Ge,
        values: vec![json!({"param": "min_units"})],
        scope: FilterScope::Row,
    });
    visual
}

fn min_units_parameter() -> Vec<Parameter> {
    vec![Parameter::new("min_units", Some(json!(1)))]
}

#[test]
fn render_uses_parameter_default_when_no_value_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let rendered = render_visual(
        &engine,
        &revenue_from_min_units(),
        &min_units_parameter(),
        &BTreeMap::new(),
    )
    .unwrap();
    // Default 1 keeps every row: North, South, West and the NULL region.
    assert_eq!(rendered.row_count, 4);
}

#[test]
fn render_applies_a_parameter_value_override() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let rendered = render_visual(
        &engine,
        &revenue_from_min_units(),
        &min_units_parameter(),
        &values(&[("min_units", "5")]),
    )
    .unwrap();
    // Only the West order has 5 or more units.
    assert_eq!(rendered.row_count, 1);
    assert!(rendered.sql.contains(">= 5"), "{}", rendered.sql);
}

#[test]
fn render_rejects_a_bad_parameter_value_and_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let err = render_visual(
        &engine,
        &revenue_from_min_units(),
        &min_units_parameter(),
        &values(&[("min_units", "lots")]),
    )
    .unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("min_units"), "{}", err.message);
}

#[test]
fn render_rejects_a_missing_required_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let mut visual = revenue_by_region();
    visual.filters.push(Filter {
        field: "region".into(),
        op: FilterOp::Eq,
        values: vec![json!({"param": "region"})],
        scope: FilterScope::Row,
    });
    // No default, so the value is required.
    let parameters = vec![Parameter::new("region", None)];
    let err = render_visual(&engine, &visual, &parameters, &BTreeMap::new()).unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("region"), "{}", err.message);

    let rendered = render_visual(
        &engine,
        &visual,
        &parameters,
        &values(&[("region", "North")]),
    )
    .unwrap();
    assert_eq!(rendered.row_count, 1);
}

#[test]
fn export_binds_parameters_like_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut report = report_with(&added, revenue_from_min_units());
    report.parameters = min_units_parameter();
    let out = dir.path().join("out");

    export_report(
        &engine,
        &report,
        ExportFormat::Csv,
        &out,
        &values(&[("min_units", "5")]),
    )
    .unwrap();
    let text = fs::read_to_string(out.join("revenue_by_region.csv")).unwrap();
    assert_eq!(
        text.lines().count(),
        2,
        "header and the West row only: {text}"
    );

    let err = export_report(
        &engine,
        &report,
        ExportFormat::Csv,
        &out,
        &values(&[("min_units", "1.5")]),
    )
    .unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("min_units"), "{}", err.message);
}

#[test]
fn unknown_parameter_value_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, _) = engine_with_sales(dir.path());
    let err = render_visual(
        &engine,
        &revenue_from_min_units(),
        &min_units_parameter(),
        &values(&[("nope", "1")]),
    )
    .unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("nope"), "{}", err.message);
}

// ---------------------------------------------------------------------------
// Calculated fields
// ---------------------------------------------------------------------------

fn field(name: &str, expression: &str) -> CalculatedField {
    CalculatedField {
        name: name.into(),
        expression: expression.into(),
        description: None,
    }
}

/// The sample's two chained fields on the sales dataset.
fn sample_fields() -> Vec<CalculatedField> {
    vec![
        field("net_revenue", "revenue * 0.9"),
        field(
            "big_ticket",
            "CASE WHEN net_revenue >= 100 THEN 'yes' ELSE 'no' END",
        ),
    ]
}

fn report_with_fields(added: &AddedDataset, fields: Vec<CalculatedField>) -> Report {
    let mut report = report_with(added, revenue_by_region());
    report.datasets[0].calculated_fields = fields;
    report
}

#[test]
fn calculated_fields_accept_sample_expressions() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(dir.path());
    let report = report_with_fields(&added, sample_fields());
    for name in ["net_revenue", "big_ticket"] {
        validate_calculated_field(&report, dir.path(), "sales", name)
            .unwrap_or_else(|e| panic!("{name}: {}", e.message));
    }
}

#[test]
fn calculated_fields_reject_subqueries_and_forbidden_functions() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(dir.path());

    let subquery = report_with_fields(&added, vec![field("bad_sub", "(SELECT 1)")]);
    let err = validate_calculated_field(&subquery, dir.path(), "sales", "bad_sub").unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("bad_sub"), "{}", err.message);

    let forbidden = report_with_fields(&added, vec![field("bad_fn", "repeat('x', 3)")]);
    let err = validate_calculated_field(&forbidden, dir.path(), "sales", "bad_fn").unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("bad_fn"), "{}", err.message);
}

#[test]
fn calculated_fields_are_checked_in_declaration_order() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(dir.path());
    // `big_ticket` uses `net_revenue`, which is defined after it here.
    let fields = vec![
        field(
            "big_ticket",
            "CASE WHEN net_revenue >= 100 THEN 'yes' ELSE 'no' END",
        ),
        field("net_revenue", "revenue * 0.9"),
    ];
    let report = report_with_fields(&added, fields);
    let err = validate_calculated_field(&report, dir.path(), "sales", "big_ticket").unwrap_err();
    assert!(err.message.contains("big_ticket"), "{}", err.message);
    // Fields are layered in order, so a later field is checked after the broken one
    // before it, and the error names that earlier field.
    let err = validate_calculated_field(&report, dir.path(), "sales", "net_revenue").unwrap_err();
    assert!(err.message.contains("big_ticket"), "{}", err.message);
    let fixed = report_with_fields(
        &added,
        vec![
            field("net_revenue", "revenue * 0.9"),
            field("big_ticket", "net_revenue > 100"),
        ],
    );
    validate_calculated_field(&fixed, dir.path(), "sales", "big_ticket").unwrap();
}

#[test]
fn validating_an_unknown_field_or_dataset_is_invalid() {
    let dir = tempfile::tempdir().unwrap();
    let (_engine, added) = engine_with_sales(dir.path());
    let report = report_with_fields(&added, sample_fields());
    let err = validate_calculated_field(&report, dir.path(), "sales", "missing").unwrap_err();
    assert_eq!(err.kind, "invalid");
    let err = validate_calculated_field(&report, dir.path(), "nope", "net_revenue").unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn dataset_with_calculated_fields_reregisters_and_exposes_new_columns() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());

    let mut dataset = added.dataset.clone();
    dataset.calculated_fields = vec![field("net_revenue", "revenue * 0.9")];
    let columns = register_dataset_fields(&engine, &dataset).unwrap();
    let net = columns.iter().find(|c| c.name == "net_revenue").unwrap();
    assert!(net.calculated);
    assert_eq!(net.role, "measure");
    let revenue = columns.iter().find(|c| c.name == "revenue").unwrap();
    assert!(!revenue.calculated);

    // A chained field is added on top; the earlier one stays.
    dataset.calculated_fields = sample_fields();
    let columns = register_dataset_fields(&engine, &dataset).unwrap();
    assert!(
        columns
            .iter()
            .any(|c| c.name == "big_ticket" && c.calculated)
    );
    assert!(
        columns
            .iter()
            .any(|c| c.name == "net_revenue" && c.calculated)
    );
    let table = preview(&engine, "sales", 5).unwrap();
    assert!(table.columns.iter().any(|c| c.name == "big_ticket"));

    // A bad edit is refused and the previous view keeps working.
    dataset
        .calculated_fields
        .push(field("broken", "(SELECT 1)"));
    let err = register_dataset_fields(&engine, &dataset).unwrap_err();
    assert_eq!(err.kind, "invalid");
    assert!(err.message.contains("broken"), "{}", err.message);
    let table = preview(&engine, "sales", 5).unwrap();
    assert!(table.columns.iter().any(|c| c.name == "big_ticket"));
    assert!(!table.columns.iter().any(|c| c.name == "broken"));
}

#[test]
fn column_views_carry_the_calculated_flag() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut dataset = added.dataset.clone();
    dataset.calculated_fields = vec![field("net_revenue", "revenue * 0.9")];
    register_dataset_fields(&engine, &dataset).unwrap();
    let columns = column_views(engine.schema("sales").unwrap());
    assert_eq!(columns.iter().filter(|c| c.calculated).count(), 1);
    assert!(
        columns
            .iter()
            .any(|c| c.name == "net_revenue" && c.calculated)
    );
}

fn row_filter(field: &str, op: FilterOp, values: Vec<serde_json::Value>) -> Filter {
    Filter {
        field: field.into(),
        op,
        values,
        scope: FilterScope::Row,
    }
}

#[test]
fn dashboard_filter_changes_numbers_only_where_the_column_exists() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let report = report_with(&added, revenue_by_region());
    let visual = &report.visuals[0];
    let none = BTreeMap::new();

    let all = render_dashboard_visual(&engine, visual, &[], &[], &none).unwrap();
    assert!(all.row_count > 1, "one row per region: {}", all.row_count);

    let north = render_dashboard_visual(
        &engine,
        visual,
        &[row_filter("region", FilterOp::Eq, vec![json!("North")])],
        &[],
        &none,
    )
    .unwrap();
    assert_eq!(north.row_count, 1, "only North is left");

    // The dataset has no such column, so the shared filter is skipped.
    let skipped = render_dashboard_visual(
        &engine,
        visual,
        &[row_filter(
            "no_such_column",
            FilterOp::Eq,
            vec![json!("Nowhere")],
        )],
        &[],
        &none,
    )
    .unwrap();
    assert_eq!(skipped.row_count, all.row_count);
}

#[test]
fn dashboard_filter_binds_parameter_values() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let report = report_with(&added, revenue_by_region());
    let params = vec![Parameter::new("region", Some(json!("North")))];
    let filter = row_filter("region", FilterOp::Eq, vec![json!({"param": "region"})]);

    let default = render_dashboard_visual(
        &engine,
        &report.visuals[0],
        std::slice::from_ref(&filter),
        &params,
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(default.row_count, 1, "the default value is North");

    let south = render_dashboard_visual(
        &engine,
        &report.visuals[0],
        std::slice::from_ref(&filter),
        &params,
        &values(&[("region", "South")]),
    )
    .unwrap();
    assert_eq!(south.row_count, 1);
    assert!(south.sql.contains("'South'"), "{}", south.sql);
}

#[test]
fn html_export_with_dashboards_draws_the_grid_and_applies_filters() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut report = report_with(&added, revenue_by_region());
    report.parameters = vec![Parameter::new("region", Some(json!("North")))];
    let mut dashboard = Dashboard::new("overview", "Overview");
    dashboard.columns = 6;
    dashboard.row_height = 40;
    dashboard.items = vec![DashboardItem {
        visual: "revenue_by_region".into(),
        x: 0,
        y: 0,
        w: 3,
        h: 2,
    }];
    dashboard.filters = vec![row_filter(
        "region",
        FilterOp::Eq,
        vec![json!({"param": "region"})],
    )];
    report.dashboards = vec![dashboard];
    let out = dir.path().join("out");

    export_report(
        &engine,
        &report,
        ExportFormat::Html,
        &out,
        &values(&[("region", "South")]),
    )
    .unwrap();
    let page = fs::read_to_string(out.join("Sample_sales.html")).unwrap();
    assert!(page.contains("repeat(6, 1fr)"), "columns in the grid");
    assert!(
        page.contains("grid-auto-rows: 40px"),
        "row height in the grid"
    );
    assert!(
        page.contains("grid-column: 1 / span 3; grid-row: 1 / span 2"),
        "placement"
    );
    assert!(
        page.contains("South"),
        "the parameter value filters the chart data"
    );
    assert!(
        !page.contains("North"),
        "the default is not used when a value is given"
    );

    // A report without dashboards keeps the plain page.
    report.dashboards.clear();
    export_report(&engine, &report, ExportFormat::Html, &out, &BTreeMap::new()).unwrap();
    let plain = fs::read_to_string(out.join("Sample_sales.html")).unwrap();
    assert!(!plain.contains("dash-grid"));
    assert!(plain.contains("<h2>Revenue by region</h2>"));
}

#[test]
fn exports_to_tableau_and_power_bi_and_imports_them_back() {
    let dir = tempfile::tempdir().unwrap();
    let (engine, added) = engine_with_sales(dir.path());
    let mut report = Report::new("Sales");
    report.sources.push(added.source);
    report.datasets.push(added.dataset);
    report.visuals.push(revenue_by_region());
    let out = dir.path().join("out");

    for (format, name) in [
        (ExportFormat::Twb, "Sales.twb"),
        (ExportFormat::Twbx, "Sales.twbx"),
        (ExportFormat::Pbit, "Sales.pbit"),
        (ExportFormat::Pbip, "Sales_pbip"),
    ] {
        let summary = export_report(&engine, &report, format, &out, &BTreeMap::new()).unwrap();
        assert!(!summary.files.is_empty());
        assert!(out.join(name).exists(), "{name}");
    }
    assert_eq!(ExportFormat::parse("pbip").unwrap(), ExportFormat::Pbip);

    // Import the Tableau workbook: a new report is saved next to it and opened.
    let (_engine, imported) = import_report(&out.join("Sales.twb")).unwrap();
    assert_eq!(imported.opened.report.visuals.len(), 1);
    assert!(out.join("Sales.dvz").exists());
    assert!(!imported.opened.datasets.is_empty());

    // A second import does not overwrite the first.
    let (_engine, again) = import_report(&out.join("Sales.twb")).unwrap();
    assert!(again.opened.path.ends_with("Sales (2).dvz"));
}

#[test]
fn importing_an_unknown_file_is_an_invalid_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x.txt");
    fs::write(&file, "nothing").unwrap();
    let err = import_report(&file).map(|_| ()).unwrap_err();
    assert_eq!(err.kind, "invalid");
}
