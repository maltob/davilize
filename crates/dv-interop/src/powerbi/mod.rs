//! Power BI files: Power BI Project folders (TMDL model + PBIR report), `.pbit`
//! templates, and the report layout of `.pbix` files.
//!
//! Import and export both go through a small intermediate description of the
//! semantic model (`Model`) and of the report pages (`PageSpec`), so the
//! parsers and writers for the different file layouts stay independent.

mod catalog;
mod datamodel;
mod export;
mod import;
mod layout;
mod model;
mod vertipaq;

use dv_core::{Aggregate, FilterOp};
use serde_json::Value;

pub(crate) use export::{export_pbip, export_pbit, export_pbix};
pub(crate) use import::import;

#[derive(Debug, Clone, Default)]
pub(crate) struct Model {
    pub tables: Vec<ModelTable>,
    pub relationships: Vec<ModelRelationship>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModelTable {
    pub name: String,
    pub columns: Vec<ModelColumn>,
    /// DAX measures: (name, expression).
    pub measures: Vec<(String, String)>,
    /// The CSV file the table loads, when its query is a plain CSV import.
    pub csv_path: Option<String>,
    /// Calculated columns that join the values of other columns into one text key,
    /// for relationships on several columns: (column name, source columns).
    pub key_columns: Vec<(String, Vec<String>)>,
    /// A table with no CSV file behind it: every column's values come from the engine
    /// (a `.pbix` only).
    pub snapshot: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelColumn {
    pub name: String,
    /// `string`, `int64`, `double`, `dateTime` or `boolean`.
    pub data_type: String,
    /// DAX expression of a calculated column.
    pub expression: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ModelRelationship {
    pub from_table: String,
    pub from_column: String,
    pub to_table: String,
    pub to_column: String,
}

/// A field used by a visual.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldRef {
    Column {
        table: String,
        column: String,
    },
    Agg {
        table: String,
        column: String,
        agg: Aggregate,
    },
    Measure {
        table: String,
        name: String,
    },
}

impl FieldRef {
    pub fn table(&self) -> &str {
        match self {
            FieldRef::Column { table, .. }
            | FieldRef::Agg { table, .. }
            | FieldRef::Measure { table, .. } => table,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct FilterSpec {
    pub field: FieldRef,
    pub op: FilterOp,
    pub values: Vec<Value>,
}

#[derive(Debug, Clone)]
pub(crate) struct VisualSpec {
    pub name: String,
    pub visual_type: String,
    pub title: Option<String>,
    /// Field wells in order: (role, field).
    pub roles: Vec<(String, FieldRef)>,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub filters: Vec<FilterSpec>,
    /// Sort field and whether it is descending.
    pub sort: Option<(FieldRef, bool)>,
}

#[derive(Debug, Clone)]
pub(crate) struct PageSpec {
    pub name: String,
    pub display_name: String,
    pub width: f64,
    pub height: f64,
    pub visuals: Vec<VisualSpec>,
}

pub(crate) fn aggregate_code(agg: Aggregate) -> u32 {
    match agg {
        Aggregate::Sum => 0,
        Aggregate::Avg => 1,
        Aggregate::Min => 2,
        Aggregate::Max => 3,
        Aggregate::Count => 4,
        Aggregate::CountDistinct => 5,
    }
}

pub(crate) fn aggregate_from_code(code: u64) -> Option<Aggregate> {
    Some(match code {
        0 => Aggregate::Sum,
        1 => Aggregate::Avg,
        2 => Aggregate::Min,
        3 => Aggregate::Max,
        4 => Aggregate::Count,
        5 => Aggregate::CountDistinct,
        _ => return None,
    })
}

pub(crate) fn aggregate_label(agg: Aggregate) -> &'static str {
    match agg {
        Aggregate::Sum => "Sum",
        Aggregate::Avg => "Avg",
        Aggregate::Min => "Min",
        Aggregate::Max => "Max",
        Aggregate::Count => "Count",
        Aggregate::CountDistinct => "CountNonNull",
    }
}

/// Decodes the UTF-16 (or plain UTF-8) text Power BI stores in `.pbit` and `.pbix` parts.
pub(crate) fn decode_text(bytes: &[u8]) -> String {
    let bom_le = bytes.starts_with(&[0xFF, 0xFE]);
    let bom_be = bytes.starts_with(&[0xFE, 0xFF]);
    let looks_utf16 = bom_le || bom_be || (bytes.len() > 3 && bytes[1] == 0 && bytes[3] == 0);
    if looks_utf16 {
        let body = if bom_le || bom_be { &bytes[2..] } else { bytes };
        let (pairs, _) = body.as_chunks::<2>();
        let units: Vec<u16> = pairs
            .iter()
            .map(|c| {
                if bom_be {
                    u16::from_be_bytes(*c)
                } else {
                    u16::from_le_bytes(*c)
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes))
            .into_owned()
    }
}

/// Encodes text as UTF-16 little endian without a byte-order mark.
pub(crate) fn encode_utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// A stable pseudo-GUID derived from a seed, for lineage tags and relationship names.
pub(crate) fn guid(seed: &str) -> String {
    let mut h: [u64; 2] = [0xcbf2_9ce4_8422_2325, 0x8422_2325_cbf2_9ce4];
    for (i, b) in seed.bytes().enumerate() {
        let k = i % 2;
        h[k] ^= u64::from(b);
        h[k] = h[k].wrapping_mul(0x0000_0100_0000_01b3);
        h[1 - k] = h[1 - k].rotate_left(5) ^ h[k];
    }
    let hex = format!("{:016x}{:016x}", h[0], h[1]);
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    )
}

#[cfg(test)]
mod tests;
