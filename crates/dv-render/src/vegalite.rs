//! Vega-Lite specs for each chart type. Pure functions: a [`Table`] and the
//! visual's encodings in, JSON out. Data is embedded as the (already aggregated
//! or capped) rows of the table.

use dv_core::{ChartType, Visual};
use serde_json::{Map, Value, json};

use crate::table::Cell;
use crate::{Channels, Field, Kind, RenderError, Table, required};

pub const SCHEMA: &str = "https://vega.github.io/schema/vega-lite/v6.json";
const WIDTH: u32 = 640;
const HEIGHT: u32 = 400;
/// Name of the row-index field added to table specs so rows keep their order.
const ROW_FIELD: &str = "__row";

/// Builds the Vega-Lite spec for a visual. `notes` go in the chart subtitle.
pub fn build_spec(visual: &Visual, table: &Table, notes: &[String]) -> Result<Value, RenderError> {
    let channels = Channels::resolve(&visual.id, &visual.encodings, table)?;
    let title = title_value(&visual.title, notes);
    match visual.chart {
        ChartType::Bar => bar(visual, table, &channels, title),
        ChartType::Line => line(visual, table, &channels, title),
        ChartType::Scatter => scatter(visual, table, &channels, title),
        ChartType::Heatmap => heatmap(visual, table, &channels, title),
        ChartType::Table => Ok(grid(table, title)),
    }
}

fn title_value(title: &str, notes: &[String]) -> Value {
    if notes.is_empty() {
        json!(title)
    } else {
        json!({"text": title, "subtitle": notes.join(" | ")})
    }
}

/// Top-level spec skeleton with the embedded data.
fn base(title: Value, data: Value) -> Map<String, Value> {
    let mut spec = Map::new();
    spec.insert("$schema".into(), json!(SCHEMA));
    // Vega-Lite turns `description` into the chart's accessible name (aria-label).
    let description = title
        .get("text")
        .unwrap_or(&title)
        .as_str()
        .map(str::to_owned);
    if let Some(description) = description {
        spec.insert("description".into(), json!(description));
    }
    spec.insert("title".into(), title);
    spec.insert("data".into(), json!({ "values": data }));
    spec.insert("width".into(), json!(WIDTH));
    spec.insert("height".into(), json!(HEIGHT));
    spec.insert("config".into(), crate::style::chart_config());
    spec
}

/// Marker shape per series, so line and scatter series differ by shape as well as
/// colour. Only category colours get shapes; a numeric colour scale keeps circles.
/// The shape range is keyed to series position, so series past the palette keep a
/// distinct shape (see `style::series_shape`).
fn shape_encoding(color: &Field) -> Option<Value> {
    if !matches!(color.kind, Kind::Nominal) {
        return None;
    }
    Some(json!({
        "field": color.name,
        "type": "nominal",
        "scale": {"range": crate::style::shape_range()}
    }))
}

/// Encoding for a field, typed by its column kind.
fn encode(field: &Field) -> Value {
    json!({"field": field.name, "type": field.kind.vega_type()})
}

/// Encoding for a categorical axis: keeps the query's order instead of sorting.
fn encode_axis(field: &Field) -> Value {
    match field.kind {
        Kind::Nominal => json!({"field": field.name, "type": "nominal", "sort": null}),
        _ => encode(field),
    }
}

fn bar(
    visual: &Visual,
    table: &Table,
    channels: &Channels,
    title: Value,
) -> Result<Value, RenderError> {
    let x = required(&visual.id, "bar", "x", &channels.x)?;
    let y = required(&visual.id, "bar", "y", &channels.y)?;
    let mut encoding = Map::new();
    encoding.insert("x".into(), encode_axis(&x));
    encoding.insert("y".into(), encode(&y));
    if let Some(color) = &channels.color {
        encoding.insert("color".into(), encode(color));
    }
    let mut spec = base(title, rows_json(table, false));
    spec.insert("mark".into(), json!({"type": "bar", "tooltip": true}));
    spec.insert("encoding".into(), Value::Object(encoding));
    Ok(Value::Object(spec))
}

fn line(
    visual: &Visual,
    table: &Table,
    channels: &Channels,
    title: Value,
) -> Result<Value, RenderError> {
    let x = required(&visual.id, "line", "x", &channels.x)?;
    let y = required(&visual.id, "line", "y", &channels.y)?;
    let mut encoding = Map::new();
    encoding.insert("x".into(), encode_axis(&x));
    encoding.insert("y".into(), encode(&y));
    if let Some(color) = &channels.color {
        encoding.insert("color".into(), encode(color));
    }
    if let Some(shape) = channels.color.as_ref().and_then(shape_encoding) {
        encoding.insert("shape".into(), shape);
    }
    let mut spec = base(title, rows_json(table, false));
    spec.insert(
        "mark".into(),
        json!({"type": "line", "point": true, "tooltip": true}),
    );
    spec.insert("encoding".into(), Value::Object(encoding));
    Ok(Value::Object(spec))
}

fn scatter(
    visual: &Visual,
    table: &Table,
    channels: &Channels,
    title: Value,
) -> Result<Value, RenderError> {
    let x = required(&visual.id, "scatter", "x", &channels.x)?;
    let y = required(&visual.id, "scatter", "y", &channels.y)?;
    let mut encoding = Map::new();
    encoding.insert("x".into(), encode(&x));
    encoding.insert("y".into(), encode(&y));
    if let Some(color) = &channels.color {
        encoding.insert("color".into(), encode(color));
    }
    if let Some(size) = &channels.size {
        encoding.insert("size".into(), encode(size));
    }
    if let Some(shape) = channels.color.as_ref().and_then(shape_encoding) {
        encoding.insert("shape".into(), shape);
    }
    let mut spec = base(title, rows_json(table, false));
    spec.insert(
        "mark".into(),
        json!({"type": "point", "filled": true, "tooltip": true}),
    );
    spec.insert("encoding".into(), Value::Object(encoding));
    Ok(Value::Object(spec))
}

fn heatmap(
    visual: &Visual,
    table: &Table,
    channels: &Channels,
    title: Value,
) -> Result<Value, RenderError> {
    let x = required(&visual.id, "heatmap", "x", &channels.x)?;
    let y = required(&visual.id, "heatmap", "y", &channels.y)?;
    let color = required(&visual.id, "heatmap", "color", &channels.color)?;
    let mut encoding = Map::new();
    encoding.insert("x".into(), encode_axis(&x));
    encoding.insert("y".into(), encode_axis(&y));
    encoding.insert(
        "color".into(),
        json!({
            "field": color.name,
            "type": "quantitative",
            "scale": {"scheme": "viridis"}
        }),
    );
    let mut spec = base(title, rows_json(table, false));
    spec.insert("mark".into(), json!({"type": "rect", "tooltip": true}));
    spec.insert("encoding".into(), Value::Object(encoding));
    Ok(Value::Object(spec))
}

/// Table as a grid: one Vega-Lite column facet per result column, each listing
/// its values top to bottom with `text` marks. Vega-Lite has no native table.
fn grid(table: &Table, title: Value) -> Value {
    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    let mut spec = Map::new();
    spec.insert("$schema".into(), json!(SCHEMA));
    spec.insert("title".into(), title);
    spec.insert("data".into(), json!({ "values": rows_json(table, true) }));
    spec.insert("repeat".into(), json!({ "column": names }));
    spec.insert(
        "spec".into(),
        json!({
            "width": 160,
            "height": {"step": 18},
            "mark": {"type": "text", "align": "left", "baseline": "middle"},
            "encoding": {
                "y": {"field": ROW_FIELD, "type": "ordinal", "axis": null, "sort": "ascending"},
                "x": {"value": 0},
                "text": {"field": {"repeat": "column"}, "type": "nominal"}
            }
        }),
    );
    Value::Object(spec)
}

/// Rows as JSON objects keyed by column name. With `row_index`, adds `__row`.
fn rows_json(table: &Table, row_index: bool) -> Value {
    let rows = table
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let mut object = Map::new();
            if row_index {
                object.insert(ROW_FIELD.into(), json!(index));
            }
            for (column, cell) in table.columns.iter().zip(row) {
                object.insert(column.name.clone(), cell_json(cell));
            }
            Value::Object(object)
        })
        .collect();
    Value::Array(rows)
}

/// JSON for one cell. Non-finite numbers were already turned into `Null`.
pub(crate) fn cell_json(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Number(n) => json!(n),
        Cell::Bool(b) => json!(b),
        Cell::Text(s) => json!(s),
        Cell::Time { text, .. } => json!(text),
    }
}
