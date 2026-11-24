use std::path::{Path, PathBuf};

use dv_core::{Aggregate, ChartType, FilterOp, FilterScope, Report};

use crate::{Format, ImportOptions, export, import};

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn load(name: &str) -> Report {
    Report::load(&examples().join(name)).unwrap()
}

fn round_trip(name: &str, format: Format) -> (Report, Vec<String>, Vec<String>) {
    let report = load(name);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(match format {
        Format::Twbx => "wb.twbx",
        _ => "wb.twb",
    });
    let exported = export(&report, &examples(), format, &file).unwrap();
    let out = dir.path().join("imported");
    let imported = import(&file, &out, &ImportOptions::default()).unwrap();
    (imported.report, exported.warnings, imported.warnings)
}

#[test]
fn joined_report_round_trips_through_twb() {
    let original = load("sample_joined.dvz");
    let (back, export_warnings, import_warnings) = round_trip("sample_joined.dvz", Format::Twb);
    assert!(export_warnings.is_empty(), "{export_warnings:?}");
    assert!(import_warnings.is_empty(), "{import_warnings:?}");

    assert_eq!(back.sources.len(), 2);
    let dataset = &back.datasets[0];
    assert_eq!(dataset.joins.len(), 1);
    assert_eq!(dataset.joins[0].kind, original.datasets[0].joins[0].kind);
    assert_eq!(dataset.joins[0].on, original.datasets[0].joins[0].on);

    let visual = &back.visuals[0];
    let before = &original.visuals[0];
    assert_eq!(visual.chart, ChartType::Bar);
    assert_eq!(visual.dimensions, before.dimensions);
    // Tableau has no place for a measure's alias, so the output name changes
    // but every reference to it stays consistent.
    assert_eq!(visual.measures.len(), before.measures.len());
    assert_eq!(visual.measures[0].field, before.measures[0].field);
    assert_eq!(visual.measures[0].agg, before.measures[0].agg);
    let out = visual.measures[0].output_name();
    assert_eq!(visual.encodings.x, before.encodings.x);
    assert_eq!(visual.encodings.y.as_deref(), Some(out.as_str()));
    assert_eq!(visual.sort.as_ref().unwrap().by, out);
}

#[test]
fn dashboards_and_parameters_round_trip() {
    let original = load("sample_dashboard.dvz");
    let (back, export_warnings, _) = round_trip("sample_dashboard.dvz", Format::Twbx);
    assert!(
        export_warnings.iter().any(|w| w.contains("shared filters")),
        "{export_warnings:?}"
    );
    assert_eq!(back.visuals.len(), original.visuals.len());
    assert_eq!(back.dashboards.len(), original.dashboards.len());
    for (a, b) in back.dashboards[0]
        .items
        .iter()
        .zip(&original.dashboards[0].items)
    {
        assert_eq!((a.x, a.w), (b.x, b.w));
    }
    // The packaged workbook carried the data along.
    assert!(matches!(
        &back.sources[0].kind,
        dv_core::SourceKind::File { path, .. } if path.to_string_lossy().contains("_data")
    ));

    let (params, ..) = round_trip("sample_params.dvz", Format::Twb);
    let want = load("sample_params.dvz");
    assert_eq!(params.parameters.len(), want.parameters.len());
    for (a, b) in params.parameters.iter().zip(&want.parameters) {
        assert_eq!(a.default, b.default);
    }
}

#[test]
fn calculated_fields_and_filters_round_trip() {
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
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("c.twb");
    export(&original, &examples(), Format::Twb, &file).unwrap();
    let back = import(&file, &dir.path().join("o"), &ImportOptions::default()).unwrap();
    let fields = &back.report.datasets[0].calculated_fields;
    assert_eq!(fields.len(), 2, "{:?}", back.warnings);
    assert_eq!(
        fields[0].expression,
        original.datasets[0].calculated_fields[0].expression
    );
    assert_eq!(
        fields[1].expression,
        original.datasets[0].calculated_fields[1].expression
    );
    for (a, b) in back.report.visuals.iter().zip(&original.visuals) {
        assert_eq!(a.filters.len(), b.filters.len(), "{}", a.id);
    }
}

const WORKBOOK: &str = r#"<?xml version='1.0' encoding='utf-8' ?>
<workbook version='18.1' xmlns:user='http://www.tableausoftware.com/xml/user'>
  <datasources>
    <datasource hasconnection='false' name='Parameters'>
      <column caption='Top N' datatype='integer' name='[Parameter 1]' param-domain-type='range' role='measure' type='quantitative' value='5'>
        <calculation class='tableau' formula='5' />
        <range max='20' min='1' />
      </column>
    </datasource>
    <datasource caption='Orders' name='federated.abc'>
      <connection class='federated'>
        <named-connections>
          <named-connection caption='data' name='textscan.1'>
            <connection class='textscan' directory='data' filename='orders.csv' />
          </named-connection>
        </named-connections>
        <relation connection='textscan.1' name='orders.csv' table='[orders#csv]' type='table' />
      </connection>
      <column caption='Size' datatype='string' name='[Calculation_1]' role='dimension' type='nominal'>
        <calculation class='tableau' formula='IF [Sales] &gt; 100 THEN &quot;big&quot; ELSE &quot;small&quot; END' />
      </column>
      <column caption='Total' datatype='real' name='[Calculation_2]' role='measure' type='quantitative'>
        <calculation class='tableau' formula='SUM([Sales])' />
      </column>
      <column caption='Running' datatype='real' name='[Calculation_3]' role='measure' type='quantitative'>
        <calculation class='tableau' formula='RUNNING_SUM(SUM([Sales]))' />
      </column>
    </datasource>
  </datasources>
  <worksheets>
    <worksheet name='Sales by size'>
      <table>
        <view>
          <datasources><datasource caption='Orders' name='federated.abc' /></datasources>
          <datasource-dependencies datasource='federated.abc'>
            <column-instance column='[Calculation_1]' derivation='None' name='[none:Calculation_1:nk]' pivot='key' type='nominal' />
            <column-instance column='[Sales]' derivation='Sum' name='[sum:Sales:qk]' pivot='key' type='quantitative' />
            <column-instance column='[Region]' derivation='None' name='[none:Region:nk]' pivot='key' type='nominal' />
          </datasource-dependencies>
          <filter class='categorical' column='[federated.abc].[none:Region:nk]'>
            <groupfilter function='union'>
              <groupfilter function='member' level='[Region]' member='&quot;East&quot;' />
              <groupfilter function='member' level='[Region]' member='&quot;West&quot;' />
            </groupfilter>
          </filter>
          <filter class='quantitative' column='[federated.abc].[sum:Sales:qk]'><min>10</min></filter>
          <computed-sort column='[federated.abc].[none:Calculation_1:nk]' direction='DESC' using='[federated.abc].[sum:Sales:qk]' />
        </view>
        <panes><pane><mark class='Bar' /></pane></panes>
        <rows>[federated.abc].[sum:Sales:qk]</rows>
        <cols>[federated.abc].[none:Calculation_1:nk]</cols>
      </table>
    </worksheet>
    <worksheet name='Pie'>
      <table>
        <view>
          <datasources><datasource name='federated.abc' /></datasources>
          <datasource-dependencies datasource='federated.abc'>
            <column-instance column='[Region]' derivation='None' name='[none:Region:nk]' pivot='key' type='nominal' />
            <column-instance column='[Sales]' derivation='Sum' name='[sum:Sales:qk]' pivot='key' type='quantitative' />
          </datasource-dependencies>
        </view>
        <panes><pane><mark class='Pie' /><encodings><color column='[federated.abc].[none:Region:nk]' /><text column='[federated.abc].[sum:Sales:qk]' /></encodings></pane></panes>
        <rows /><cols />
      </table>
    </worksheet>
  </worksheets>
  <dashboards>
    <dashboard name='Overview'>
      <size maxheight='400' maxwidth='1000' minheight='400' minwidth='1000' />
      <zones>
        <zone h='100000' id='1' type-v2='layout-basic' w='100000' x='0' y='0'>
          <zone h='100000' id='2' name='Sales by size' w='50000' x='0' y='0' />
          <zone h='100000' id='3' name='Pie' w='50000' x='50000' y='0' />
        </zone>
      </zones>
    </dashboard>
  </dashboards>
</workbook>"#;

#[test]
fn imports_a_hand_written_workbook() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("orders.twb");
    std::fs::write(&file, WORKBOOK).unwrap();
    let imported = import(&file, dir.path(), &ImportOptions::default()).unwrap();
    let report = &imported.report;

    assert_eq!(report.parameters.len(), 1);
    assert_eq!(report.parameters[0].name, "Top_N");
    assert_eq!(report.parameters[0].default, Some(serde_json::json!(5)));
    assert_eq!(report.parameters[0].max, Some(serde_json::json!(20)));

    let dataset = &report.datasets[0];
    assert_eq!(dataset.calculated_fields.len(), 1);
    assert_eq!(
        dataset.calculated_fields[0].expression,
        "CASE WHEN \"Sales\" > 100 THEN 'big' ELSE 'small' END"
    );
    assert!(
        imported
            .warnings
            .iter()
            .any(|w| w.contains("Running") && w.contains("not imported")),
        "{:?}",
        imported.warnings
    );

    let sheet = report
        .visuals
        .iter()
        .find(|v| v.title == "Sales by size")
        .unwrap();
    assert_eq!(sheet.chart, ChartType::Bar);
    assert_eq!(sheet.encodings.x.as_deref(), Some("Size"));
    assert_eq!(sheet.encodings.y.as_deref(), Some("sum_Sales"));
    assert_eq!(sheet.measures[0].agg, Aggregate::Sum);
    let region = sheet.filters.iter().find(|f| f.field == "Region").unwrap();
    assert_eq!(region.op, FilterOp::In);
    assert_eq!(region.values.len(), 2);
    let sales = sheet
        .filters
        .iter()
        .find(|f| f.field == "sum_Sales")
        .unwrap();
    assert_eq!(
        (sales.op, sales.scope),
        (FilterOp::Ge, FilterScope::Aggregate)
    );
    assert_eq!(sheet.sort.as_ref().unwrap().by, "sum_Sales");

    let pie = report.visuals.iter().find(|v| v.title == "Pie").unwrap();
    assert_eq!(pie.chart, ChartType::Bar);
    assert!(imported.warnings.iter().any(|w| w.contains("pie chart")));

    let dashboard = &report.dashboards[0];
    assert_eq!(dashboard.items.len(), 2);
    assert_eq!((dashboard.items[0].x, dashboard.items[0].w), (0, 6));
    assert_eq!((dashboard.items[1].x, dashboard.items[1].w), (6, 6));
    assert_eq!(dashboard.items[0].h, 5);
}

#[test]
fn the_expression_check_can_reject_a_field() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("orders.twb");
    std::fs::write(&file, WORKBOOK).unwrap();
    let reject = |_: &str, _: &str| Err("nope".to_string());
    let options = ImportOptions {
        check_expression: Some(&reject),
    };
    let imported = import(&file, dir.path(), &options).unwrap();
    assert!(imported.report.datasets[0].calculated_fields.is_empty());
    assert!(imported.warnings.iter().any(|w| w.contains("nope")));
}

#[test]
fn rejects_files_that_are_not_workbooks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bad.twb");
    std::fs::write(&file, "<html/>").unwrap();
    assert!(import(&file, dir.path(), &ImportOptions::default()).is_err());
}
