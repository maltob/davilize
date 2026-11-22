use std::sync::Arc;

use duckdb::arrow::array::{ArrayRef, Date32Array, Float64Array, Int64Array, StringArray};
use duckdb::arrow::record_batch::RecordBatch;
use dv_core::{Aggregate, ChartType, Encodings, Measure, Visual};
use serde_json::{Value, json};

use crate::html::{Scripts, escape_html, html_report, json_for_script};
use crate::table::{Sampling, Table, excel_serial};
use crate::vegalite::SCHEMA;
use crate::{RenderError, prepare, prepare_with_cap, raster, svg, xlsx};

fn visual(id: &str, chart: ChartType, encodings: Encodings) -> Visual {
    Visual {
        id: id.into(),
        title: format!("Title {id}"),
        dataset: "sales".into(),
        chart,
        encodings,
        dimensions: vec![],
        measures: vec![],
        filters: vec![],
        sort: None,
        limit: None,
    }
}

fn enc(x: &str, y: &str) -> Encodings {
    Encodings {
        x: Some(x.into()),
        y: Some(y.into()),
        color: None,
        size: None,
    }
}

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

fn regions() -> RecordBatch {
    batch(vec![
        (
            "region",
            Arc::new(StringArray::from(vec!["North", "South"])) as ArrayRef,
        ),
        (
            "total_revenue",
            Arc::new(Float64Array::from(vec![150.0, 60.0])) as ArrayRef,
        ),
    ])
}

#[test]
fn bar_spec_matches_golden() {
    let v = Visual {
        title: "Revenue by region".into(),
        ..visual(
            "revenue_by_region",
            ChartType::Bar,
            enc("region", "total_revenue"),
        )
    };
    let rendered = prepare(&v, &[regions()]).unwrap();
    let expected = json!({
        "$schema": SCHEMA,
        "title": "Revenue by region",
        "data": {"values": [
            {"region": "North", "total_revenue": 150.0},
            {"region": "South", "total_revenue": 60.0}
        ]},
        "width": 640,
        "height": 400,
        "mark": {"type": "bar", "tooltip": true},
        "encoding": {
            "x": {"field": "region", "type": "nominal", "sort": null},
            "y": {"field": "total_revenue", "type": "quantitative"}
        }
    });
    assert_eq!(rendered.spec, expected);
    assert!(rendered.notes.is_empty());
}

#[test]
fn line_spec_adds_colour_and_points() {
    let batches = [batch(vec![
        (
            "day",
            Arc::new(StringArray::from(vec!["Mon", "Mon", "Tue"])) as ArrayRef,
        ),
        (
            "v",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "g",
            Arc::new(StringArray::from(vec!["a", "b", "a"])) as ArrayRef,
        ),
    ])];
    let mut encodings = enc("day", "v");
    encodings.color = Some("g".into());
    let v = visual("line", ChartType::Line, encodings);
    let spec = prepare(&v, &batches).unwrap().spec;
    assert_eq!(
        spec["mark"],
        json!({"type": "line", "point": true, "tooltip": true})
    );
    assert_eq!(
        spec["encoding"]["color"],
        json!({"field": "g", "type": "nominal"})
    );
}

#[test]
fn scatter_downsamples_evenly_over_the_cap_and_notes_it() {
    let n = 100usize;
    let xs: Vec<f64> = (0..n).map(|i| i as f64).collect();
    let batches = [batch(vec![
        ("x", Arc::new(Float64Array::from(xs.clone())) as ArrayRef),
        ("y", Arc::new(Float64Array::from(xs)) as ArrayRef),
    ])];
    let v = visual("scatter", ChartType::Scatter, enc("x", "y"));
    let rendered = prepare_with_cap(&v, &batches, 10).unwrap();
    assert_eq!(rendered.table.total_rows, 100);
    assert_eq!(rendered.table.rows.len(), 10);
    // Indices are i * 100 / 10, so the sample is 0, 10, ..., 90.
    let picked: Vec<f64> = rendered
        .table
        .rows
        .iter()
        .map(|r| match r[0] {
            crate::Cell::Number(v) => v,
            _ => panic!("expected number"),
        })
        .collect();
    assert_eq!(picked, (0..10).map(|i| (i * 10) as f64).collect::<Vec<_>>());
    assert_eq!(
        rendered.notes,
        vec!["Showing 10 of 100 points (evenly spaced sample, row cap 10)".to_string()]
    );
    assert_eq!(
        rendered.spec["title"]["subtitle"],
        json!(rendered.notes.join(" | "))
    );
}

#[test]
fn aggregated_charts_keep_the_first_rows_when_capped() {
    let batches = [regions()];
    let table = Table::from_batches(&batches, 1, Sampling::Head).unwrap();
    assert_eq!(table.rows.len(), 1);
    assert_eq!(table.rows[0][0], crate::Cell::Text("North".into()));
}

#[test]
fn table_grid_repeats_every_column_and_keeps_row_order() {
    let v = visual("grid", ChartType::Table, Encodings::default());
    let rendered = prepare(&v, &[regions()]).unwrap();
    assert_eq!(
        rendered.spec["repeat"]["column"],
        json!(["region", "total_revenue"])
    );
    assert_eq!(rendered.spec["data"]["values"][1]["__row"], json!(1));
    assert_eq!(
        rendered.spec["spec"]["encoding"]["text"]["field"],
        json!({"repeat": "column"})
    );
}

#[test]
fn heatmap_needs_a_colour_measure() {
    let v = visual("heat", ChartType::Heatmap, enc("region", "total_revenue"));
    let err = prepare(&v, &[regions()]).unwrap_err();
    assert!(matches!(
        err,
        RenderError::MissingEncoding {
            chart: "heatmap",
            channel: "color",
            ..
        }
    ));
}

#[test]
fn unknown_field_is_reported_with_the_visual_id() {
    let v = visual("bad", ChartType::Bar, enc("nope", "total_revenue"));
    let err = prepare(&v, &[regions()]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "visual 'bad': field 'nope' is not in the query result"
    );
}

#[test]
fn table_uses_all_result_columns() {
    // A grouped visual's columns are its dimensions followed by measure aliases.
    let mut v = visual("g", ChartType::Table, Encodings::default());
    v.measures = vec![Measure {
        field: "revenue".into(),
        agg: Aggregate::Sum,
        alias: Some("total_revenue".into()),
    }];
    let rendered = prepare(&v, &[regions()]).unwrap();
    assert_eq!(rendered.table.columns.len(), 2);
}

#[test]
fn temporal_columns_get_excel_serials() {
    let batches = [batch(vec![
        ("day", Arc::new(Date32Array::from(vec![0, 1])) as ArrayRef),
        ("v", Arc::new(Int64Array::from(vec![1, 2])) as ArrayRef),
    ])];
    let table = Table::from_batches(&batches, 10, Sampling::Head).unwrap();
    match &table.rows[0][0] {
        crate::Cell::Time { serial, .. } => assert_eq!(*serial, 25569.0),
        other => panic!("expected a time cell, got {other:?}"),
    }
    assert_eq!(
        excel_serial(&duckdb::arrow::datatypes::DataType::Date32, 1),
        25570.0
    );
}

#[test]
fn script_json_cannot_close_the_script_element() {
    let value = json!({"t": "</script><script>alert(1)</script> <!-- &"});
    let text = json_for_script(&value);
    assert!(!text.contains('<'));
    assert!(!text.contains('>'));
    assert!(!text.contains('&'));
    assert!(text.contains("\\u003c/script\\u003e"));
}

#[test]
fn html_escapes_titles_and_keeps_data_out_of_markup() {
    let mut v = visual("evil", ChartType::Bar, enc("region", "total_revenue"));
    v.title = "<img src=x onerror=alert(1)> & \"quotes\"".into();
    let batches = [batch(vec![
        (
            "region",
            Arc::new(StringArray::from(vec![
                "</script><script>alert(2)</script>",
            ])) as ArrayRef,
        ),
        (
            "total_revenue",
            Arc::new(Float64Array::from(vec![1.0])) as ArrayRef,
        ),
    ])];
    let rendered = prepare(&v, &batches).unwrap();
    let page = html_report("Report <b>", &[rendered], &Scripts::Cdn).unwrap();

    assert!(page.starts_with("<!doctype html>"));
    assert!(page.contains("&lt;img src=x onerror=alert(1)&gt; &amp; &quot;quotes&quot;"));
    assert!(page.contains("<title>Report &lt;b&gt;</title>"));
    assert!(!page.contains("<img src=x"));
    assert!(!page.contains("alert(2)</script>"));
    // Three library tags and one inline block: exactly four closing tags.
    assert_eq!(page.matches("</script>").count(), 4);
    assert!(page.contains("cdn.jsdelivr.net/npm/vega@6.4.0"));
    assert_eq!(escape_html("'"), "&#x27;");
}

#[test]
fn html_inline_mode_rejects_vendored_markup_that_would_break_out() {
    let rendered = prepare(
        &visual("v", ChartType::Bar, enc("region", "total_revenue")),
        &[regions()],
    )
    .unwrap();
    let err = html_report(
        "r",
        &[rendered],
        &Scripts::Inline {
            vega: "var a = '</SCRIPT>';",
            vega_lite: "",
            vega_embed: "",
        },
    )
    .unwrap_err();
    assert!(matches!(err, RenderError::Output(_)));
}

#[test]
fn svg_draws_bars_and_escapes_text() {
    let mut v = visual("bars", ChartType::Bar, enc("region", "total_revenue"));
    v.title = "A & B".into();
    let rendered = prepare(&v, &[regions()]).unwrap();
    let text = svg::render(&rendered).unwrap();
    assert!(text.starts_with("<svg "));
    assert!(text.contains("A &amp; B"));
    // One white background rect plus one rect per bar.
    assert_eq!(text.matches("<rect x=").count(), 3);
    assert!(text.trim_end().ends_with("</svg>"));
}

#[test]
fn svg_colours_series_alphabetically_like_vega_lite() {
    // Rows arrive West, North, South; Vega-Lite colours North, South, West in
    // that order, so North must get the first palette colour here too.
    let data = batch(vec![
        (
            "product",
            Arc::new(StringArray::from(vec!["a", "a", "a"])) as ArrayRef,
        ),
        (
            "total_revenue",
            Arc::new(Float64Array::from(vec![3.0, 2.0, 1.0])) as ArrayRef,
        ),
        (
            "region",
            Arc::new(StringArray::from(vec!["West", "North", "South"])) as ArrayRef,
        ),
    ]);
    let mut e = enc("product", "total_revenue");
    e.color = Some("region".into());
    let v = visual("series", ChartType::Line, e);
    let text = svg::render(&prepare(&v, &[data]).unwrap()).unwrap();
    let legend = |name: &str| {
        let at = text.find(&format!(">{name}</text>")).expect(name);
        let rect = text[..at].rfind("<rect").unwrap();
        let fill = text[rect..].find("fill=\"#").unwrap() + rect + 6;
        text[fill..fill + 7].to_string()
    };
    assert_eq!(legend("North"), "#4e79a7");
    assert_eq!(legend("South"), "#f28e2b");
    assert_eq!(legend("West"), "#e15759");
}

fn heat_data() -> RecordBatch {
    batch(vec![
        (
            "region",
            Arc::new(StringArray::from(vec!["West", "West", "East", "East"])) as ArrayRef,
        ),
        (
            "product",
            Arc::new(StringArray::from(vec!["a<b", "c", "a<b", "c"])) as ArrayRef,
        ),
        (
            "total",
            Arc::new(Float64Array::from(vec![0.0, 50.0, 100.0, 100.0])) as ArrayRef,
        ),
    ])
}

fn heat_visual() -> Visual {
    let mut e = enc("region", "product");
    e.color = Some("total".into());
    visual("h", ChartType::Heatmap, e)
}

#[test]
fn svg_heatmap_colours_cells_on_a_viridis_scale_with_a_legend() {
    let text = svg::render(&prepare(&heat_visual(), &[heat_data()]).unwrap()).unwrap();
    assert!(text.starts_with("<svg "));
    // Lowest value is the first viridis stop, highest is the last.
    assert!(text.contains("fill=\"#440154\""), "min colour missing");
    assert!(text.contains("fill=\"#fde725\""), "max colour missing");
    // Gradient legend with both ends labelled.
    assert!(text.contains("linearGradient") && text.contains("url(#heat)"));
    assert!(text.contains(">100</text>") && text.contains(">0</text>"));
    // Axis labels are escaped, never raw markup.
    assert!(text.contains("a&lt;b") && !text.contains("a<b"));
    // Background + 4 cells + legend bar.
    assert_eq!(text.matches("<rect x=").count(), 6);
}

#[test]
fn svg_heatmap_needs_a_colour_measure() {
    let v = visual("h", ChartType::Heatmap, enc("region", "total_revenue"));
    let rendered = prepare(&v, &[regions()]);
    // Either prepare or render must refuse it, with the visual named.
    let err = match rendered {
        Err(e) => e,
        Ok(r) => svg::render(&r).unwrap_err(),
    };
    assert!(err.to_string().contains('h'), "{err}");
}

#[test]
fn svg_draws_a_table_grid_with_header_and_escaped_cells() {
    let data = batch(vec![
        (
            "name",
            Arc::new(StringArray::from(vec!["a & b", "<script>"])) as ArrayRef,
        ),
        (
            "total",
            Arc::new(Float64Array::from(vec![1.5, 20.0])) as ArrayRef,
        ),
    ]);
    let mut v = visual("t", ChartType::Table, Encodings::default());
    v.title = "People".into();
    let text = svg::render(&prepare(&v, &[data]).unwrap()).unwrap();
    assert!(text.starts_with("<svg "));
    assert!(text.contains(">name</text>") && text.contains(">total</text>"));
    assert!(text.contains("a &amp; b") && text.contains("&lt;script&gt;"));
    assert!(!text.contains("<script>"));
    // Numbers are right-aligned, text left-aligned.
    assert!(text.contains("text-anchor=\"end\" fill=\"#1f2933\">20<"));
    assert!(text.trim_end().ends_with("</svg>"));
}

#[test]
fn svg_table_caps_rows_and_says_so() {
    let n = 100;
    let data = batch(vec![(
        "n",
        Arc::new(Int64Array::from((0..n).collect::<Vec<i64>>())) as ArrayRef,
    )]);
    let v = visual("t", ChartType::Table, Encodings::default());
    let text = svg::render(&prepare(&v, &[data]).unwrap()).unwrap();
    assert!(text.contains("Showing the first 40 of 100 rows."));
}

#[test]
fn xlsx_is_a_zip_with_one_sheet_per_visual() {
    let a = prepare(
        &visual("first", ChartType::Bar, enc("region", "total_revenue")),
        &[regions()],
    )
    .unwrap();
    let b = prepare(
        &visual("second", ChartType::Table, Encodings::default()),
        &[regions()],
    )
    .unwrap();
    let bytes = xlsx::workbook(&[a, b]).unwrap();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    // Zip entry names are stored uncompressed: one worksheet part per visual.
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("xl/worksheets/sheet1.xml"));
    assert!(text.contains("xl/worksheets/sheet2.xml"));
    assert!(!text.contains("xl/worksheets/sheet3.xml"));
}

#[test]
fn png_and_pdf_have_their_file_signatures() {
    let numeric = batch(vec![
        (
            "x",
            Arc::new(Float64Array::from(vec![1.0, 2.0, 3.0])) as ArrayRef,
        ),
        (
            "y",
            Arc::new(Float64Array::from(vec![2.0, 1.0, 3.0])) as ArrayRef,
        ),
    ]);
    let rendered = prepare(&visual("p", ChartType::Scatter, enc("x", "y")), &[numeric]).unwrap();
    let svg_text = svg::render(&rendered).unwrap();
    let png = raster::png(&svg_text).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.len() > 1000);
    let pdf = raster::pdf(&svg_text).unwrap();
    assert_eq!(&pdf[..5], b"%PDF-");
}

#[test]
fn spec_builders_are_pure_json() {
    // A spec must survive a JSON round trip unchanged (no non-finite numbers).
    let rendered = prepare(
        &visual("j", ChartType::Bar, enc("region", "total_revenue")),
        &[regions()],
    )
    .unwrap();
    let text = serde_json::to_string(&rendered.spec).unwrap();
    let back: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(back, rendered.spec);
}
