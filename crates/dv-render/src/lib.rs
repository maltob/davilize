//! Report rendering for Davilize.
//!
//! A visual's query result is turned into a [`Rendered`] value once. From it
//! the output modules produce:
//!
//! * a Vega-Lite spec (JSON) for every chart type, embedded in the HTML report;
//! * a native SVG for every chart type ([`svg`]);
//! * PNG and PDF rasterised or converted from that SVG ([`raster`]);
//! * an XLSX workbook with one typed sheet per visual ([`xlsx`]).
//!
//! Spec generation is pure: it reads the [`Table`] and returns JSON, so it can be
//! tested without touching files or fonts.

pub mod dashboard;
pub mod html;
pub mod raster;
pub mod svg;
pub mod table;
pub mod vegalite;
pub mod xlsx;

use duckdb::arrow::record_batch::RecordBatch;
use dv_core::{ChartType, Encodings, Visual};
use serde_json::Value;

pub use dashboard::dashboard_views;
pub use html::{ChartPlacement, DashboardView, Scripts, html_dashboards, html_report};
pub use table::{Cell, Column, Kind, Sampling, Table};

/// Rows embedded in a spec or rendered per visual before the cap applies.
pub const MAX_ROWS: usize = 50_000;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("visual '{visual}': field '{field}' is not in the query result")]
    MissingField { visual: String, field: String },
    #[error("visual '{visual}': {chart} charts need the '{channel}' encoding")]
    MissingEncoding {
        visual: String,
        chart: &'static str,
        channel: &'static str,
    },
    #[error("visual '{visual}': {reason}")]
    Unsupported { visual: String, reason: String },
    #[error("{0}")]
    Output(String),
}

/// A visual ready for output: its typed rows, notes about capping and its Vega-Lite spec.
#[derive(Debug, Clone)]
pub struct Rendered {
    pub id: String,
    pub title: String,
    pub chart: ChartType,
    pub encodings: Encodings,
    pub table: Table,
    /// Human-readable notes, e.g. about downsampling. Shown under the chart.
    pub notes: Vec<String>,
    pub spec: Value,
}

/// Builds the render model for `visual` from its query result, using [`MAX_ROWS`].
pub fn prepare(visual: &Visual, batches: &[RecordBatch]) -> Result<Rendered, RenderError> {
    prepare_with_cap(visual, batches, MAX_ROWS)
}

/// Like [`prepare`] with an explicit row cap (tests use small caps).
pub fn prepare_with_cap(
    visual: &Visual,
    batches: &[RecordBatch],
    cap: usize,
) -> Result<Rendered, RenderError> {
    let sampling = match visual.chart {
        ChartType::Scatter => Sampling::Stride,
        _ => Sampling::Head,
    };
    let table = Table::from_batches(batches, cap, sampling)?;
    let mut notes = Vec::new();
    if table.dropped_rows() > 0 {
        let kept = table.rows.len();
        let total = table.total_rows;
        notes.push(match sampling {
            Sampling::Stride => {
                format!("Showing {kept} of {total} points (evenly spaced sample, row cap {cap})")
            }
            Sampling::Head => format!("Showing first {kept} of {total} rows (row cap {cap})"),
        });
    }
    let spec = vegalite::build_spec(visual, &table, &notes)?;
    Ok(Rendered {
        id: visual.id.clone(),
        title: visual.title.clone(),
        chart: visual.chart,
        encodings: visual.encodings.clone(),
        table,
        notes,
        spec,
    })
}

/// A channel field resolved against the table.
#[derive(Debug, Clone)]
pub(crate) struct Field {
    pub name: String,
    pub kind: Kind,
}

/// The encodings of a visual, resolved against the table's columns.
#[derive(Debug, Clone, Default)]
pub(crate) struct Channels {
    pub x: Option<Field>,
    pub y: Option<Field>,
    pub color: Option<Field>,
    pub size: Option<Field>,
}

impl Channels {
    pub fn resolve(
        id: &str,
        encodings: &Encodings,
        table: &Table,
    ) -> Result<Channels, RenderError> {
        Ok(Channels {
            x: resolve_one(id, table, &encodings.x)?,
            y: resolve_one(id, table, &encodings.y)?,
            color: resolve_one(id, table, &encodings.color)?,
            size: resolve_one(id, table, &encodings.size)?,
        })
    }
}

fn resolve_one(
    id: &str,
    table: &Table,
    name: &Option<String>,
) -> Result<Option<Field>, RenderError> {
    let Some(name) = name else {
        return Ok(None);
    };
    // An empty result has no columns; there is nothing to check against yet.
    if table.columns.is_empty() {
        return Ok(Some(Field {
            name: name.clone(),
            kind: Kind::Nominal,
        }));
    }
    match table.columns.iter().find(|c| &c.name == name) {
        Some(column) => Ok(Some(Field {
            name: column.name.clone(),
            kind: column.kind,
        })),
        None => Err(RenderError::MissingField {
            visual: id.to_string(),
            field: name.clone(),
        }),
    }
}

/// Returns the required channel or an error naming the chart type.
pub(crate) fn required(
    id: &str,
    chart: &'static str,
    channel: &'static str,
    field: &Option<Field>,
) -> Result<Field, RenderError> {
    field.clone().ok_or_else(|| RenderError::MissingEncoding {
        visual: id.to_string(),
        chart,
        channel,
    })
}

#[cfg(test)]
mod tests;
