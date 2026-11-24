use std::io::Write;
use std::path::{Path, PathBuf};

use dv_core::{ChartType, Report};

use crate::{Format, ImportOptions, export, import};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn load(name: &str) -> Report {
    Report::load(&examples().join(name)).unwrap()
}

struct Trip {
    back: Report,
    export_warnings: Vec<String>,
    import_warnings: Vec<String>,
    _dir: tempfile::TempDir,
}

fn round_trip(name: &str, format: Format) -> Trip {
    let report = load(name);
    let dir = tempfile::tempdir().unwrap();
    let target = match format {
        Format::Pbit => dir.path().join("out.pbit"),
        _ => dir.path().join("project"),
    };
    let exported = export(&report, &examples(), format, &target).unwrap();
    let imported = import(
        &target,
        &dir.path().join("imported"),
        &ImportOptions::default(),
    )
    .unwrap();
    Trip {
        back: imported.report,
        export_warnings: exported.warnings,
        import_warnings: imported.warnings,
        _dir: dir,
    }
}

#[test]
fn joined_report_round_trips_through_a_project_folder() {
    let original = load("sample_joined.dvz");
    let t = round_trip("sample_joined.dvz", Format::Pbip);
    assert!(t.export_warnings.is_empty(), "{:?}", t.export_warnings);
    assert!(t.import_warnings.is_empty(), "{:?}", t.import_warnings);

    // Two tables, and the join came back as a relationship.
    assert_eq!(t.back.datasets.len(), 2);
    let sales = t
        .back
        .datasets
        .iter()
        .find(|d| d.name == "sales_managed")
        .unwrap();
    assert_eq!(sales.joins.len(), 1);
    assert_eq!(sales.joins[0].on, original.datasets[0].joins[0].on);

    let visual = &t.back.visuals[0];
    let before = &original.visuals[0];
    assert_eq!(visual.dataset, "sales_managed");
    assert_eq!(visual.chart, ChartType::Bar);
    assert_eq!(visual.dimensions, before.dimensions);
    assert_eq!(visual.measures[0].field, before.measures[0].field);
    assert_eq!(visual.measures[0].agg, before.measures[0].agg);
    let out = visual.measures[0].output_name();
    assert_eq!(visual.encodings.x, before.encodings.x);
    assert_eq!(visual.encodings.y.as_deref(), Some(out.as_str()));
    assert_eq!(visual.sort.as_ref().unwrap().by, out);
    assert_eq!(
        visual.sort.as_ref().unwrap().direction,
        before.sort.as_ref().unwrap().direction
    );
}

#[test]
fn the_project_folder_has_the_expected_layout() {
    let report = load("sample_dashboard.dvz");
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("p");
    export(&report, &examples(), Format::Pbip, &out).unwrap();
    let name = crate::util::sanitize(&report.name);
    assert!(out.join(format!("{name}.pbip")).is_file());
    assert!(
        out.join(format!("{name}.SemanticModel/definition/model.tmdl"))
            .is_file()
    );
    assert!(
        out.join(format!("{name}.SemanticModel/definition/tables"))
            .is_dir()
    );
    assert!(
        out.join(format!("{name}.Report/definition/pages/pages.json"))
            .is_file()
    );
    assert!(out.join(format!("{name}.Report/definition.pbir")).is_file());
}

#[test]
fn dashboards_and_filters_round_trip_through_a_template() {
    let original = load("sample_dashboard.dvz");
    let t = round_trip("sample_dashboard.dvz", Format::Pbit);
    // A visual shown on two dashboards becomes one visual per page in Power BI.
    assert!(t.back.visuals.len() >= original.visuals.len());
    assert_eq!(t.back.dashboards.len(), original.dashboards.len());
    for (a, b) in t.back.dashboards[0]
        .items
        .iter()
        .zip(&original.dashboards[0].items)
    {
        assert_eq!((a.x, a.w), (b.x, b.w));
    }
    // Shared dashboard filters are applied to each visual in Power BI.
    for b in &original.visuals {
        let a = t.back.visuals.iter().find(|v| v.title == b.title).unwrap();
        assert!(a.filters.len() >= b.filters.len(), "{}", a.id);
    }
}

#[test]
fn calculated_fields_become_calculated_columns() {
    let dir = tempfile::tempdir().unwrap();
    let mut original = load("sample.dvz");
    original.datasets[0].calculated_fields = vec![
        dv_core::CalculatedField {
            name: "margin".into(),
            expression: "\"revenue\" - \"units\"".into(),
            description: None,
        },
        dv_core::CalculatedField {
            name: "size".into(),
            expression: "CASE WHEN \"units\" > 3 THEN 'big' ELSE 'small' END".into(),
            description: None,
        },
    ];
    let file = dir.path().join("c.pbit");
    export(&original, &examples(), Format::Pbit, &file).unwrap();
    let back = import(&file, &dir.path().join("o"), &ImportOptions::default()).unwrap();
    let fields = &back.report.datasets[0].calculated_fields;
    assert_eq!(fields.len(), 2, "{:?}", back.warnings);
    assert_eq!(fields[0].expression, "\"revenue\" - \"units\"");
    assert_eq!(
        fields[1].expression,
        original.datasets[0].calculated_fields[1].expression
    );
}

#[test]
fn a_pbix_imports_its_layout_with_placeholder_sources() {
    let report = load("sample_joined.dvz");
    let dir = tempfile::tempdir().unwrap();
    let pbit = dir.path().join("a.pbit");
    export(&report, &examples(), Format::Pbit, &pbit).unwrap();

    // A .pbix has the layout but a binary model, so drop the schema.
    let pbix = dir.path().join("a.pbix");
    {
        let mut src = zip::ZipArchive::new(std::fs::File::open(&pbit).unwrap()).unwrap();
        let mut out = zip::ZipWriter::new(std::fs::File::create(&pbix).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for i in 0..src.len() {
            let mut entry = src.by_index(i).unwrap();
            let name = entry
                .enclosed_name()
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if name == "DataModelSchema" {
                continue;
            }
            out.start_file(name, options).unwrap();
            std::io::copy(&mut entry, &mut out).unwrap();
        }
        out.start_file("DataModel", options).unwrap();
        out.write_all(&[0, 1, 2, 3]).unwrap();
        out.finish().unwrap();
    }
    let imported = import(&pbix, &dir.path().join("o"), &ImportOptions::default()).unwrap();
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("compressed binary"))
    );
    assert_eq!(imported.report.visuals.len(), 1);
    assert!(!imported.report.sources.is_empty());
    imported.report.validate().unwrap();
}

#[test]
fn writing_a_pbix_is_refused_with_advice() {
    let report = load("sample.dvz");
    let dir = tempfile::tempdir().unwrap();
    let err = export(
        &report,
        &examples(),
        Format::Pbix,
        &dir.path().join("x.pbix"),
    )
    .unwrap_err();
    assert!(err.to_string().contains(".pbit"));
}

#[test]
fn tmdl_tables_parse_back() {
    use super::model::{parse_tmdl_table, tmdl_table};
    use super::{ModelColumn, ModelTable};
    let table = ModelTable {
        name: "Sales Data".into(),
        columns: vec![
            ModelColumn {
                name: "region".into(),
                data_type: "string".into(),
                expression: None,
            },
            ModelColumn {
                name: "margin".into(),
                data_type: "double".into(),
                expression: Some("[revenue] - [cost]".into()),
            },
        ],
        measures: vec![("Total".into(), "SUM('Sales Data'[revenue])".into())],
        csv_path: None,
    };
    let query = vec![
        "let".to_string(),
        "    Source = Csv.Document(File.Contents(\"C:/d/x.csv\"))".into(),
        "in".into(),
        "    Source".into(),
    ];
    let parsed = parse_tmdl_table(&tmdl_table(&table, Some(&query))).unwrap();
    assert_eq!(parsed.name, "Sales Data");
    assert_eq!(parsed.columns.len(), 2);
    assert_eq!(
        parsed.columns[1].expression.as_deref(),
        Some("[revenue] - [cost]")
    );
    assert_eq!(parsed.measures[0].0, "Total");
    assert_eq!(parsed.csv_path.as_deref(), Some("C:/d/x.csv"));
}
