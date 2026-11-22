use std::fs;
use std::path::{Path, PathBuf};

use dv_core::{Aggregate, ChartType, Encodings, Measure, Report, Visual};

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
    let rendered = render_visual(&engine, &revenue_by_region()).unwrap();
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
    let err = render_visual(&engine, &visual).unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn render_visual_reports_missing_dataset_as_query_error() {
    let engine = Engine::new().unwrap();
    let err = render_visual(&engine, &revenue_by_region()).unwrap_err();
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
    let summary = export_report(&engine, &report, ExportFormat::Csv, &out).unwrap();
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

    let html = export_report(&engine, &report, ExportFormat::Html, &out).unwrap();
    assert_eq!(html.files.len(), 1);
    let page = fs::read_to_string(out.join("Sample_sales.html")).unwrap();
    assert!(page.contains("<h2>Revenue by region</h2>"));
    assert!(page.contains("vega-embed"));

    export_report(&engine, &report, ExportFormat::Xlsx, &out).unwrap();
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

    let summary = export_report(&engine, &report, ExportFormat::Png, &out).unwrap();
    assert!(summary.warnings.is_empty());
    let image = fs::read(out.join("revenue_by_region.png")).unwrap();
    assert!(image.starts_with(&[0x89, b'P', b'N', b'G']));

    let mut report = report_with(&added, table);
    report.visuals[0].id = "as_table".into();
    let summary = export_report(&engine, &report, ExportFormat::Png, &out).unwrap();
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
    let err =
        export_report(&engine, &report, ExportFormat::Csv, &dir.path().join("out")).unwrap_err();
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
    let rendered = render_visual(&engine, &opened.report.visuals[0]).unwrap();
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
    let rendered = render_visual(&engine, &reopened.report.visuals[0]).unwrap();
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
