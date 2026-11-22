//! Typed, row-capped table built from query results. Every renderer (Vega-Lite
//! JSON, native SVG, XLSX) reads this instead of Arrow directly, so they agree
//! on types and on which rows are shown.

use duckdb::arrow::array::{Array, ArrayRef, AsArray};
use duckdb::arrow::compute::cast;
use duckdb::arrow::datatypes::{DataType, Float64Type, Int64Type, TimeUnit};
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::display::array_value_to_string;

use crate::RenderError;

/// Days from the Excel epoch (1899-12-30) to the Unix epoch.
const EXCEL_UNIX_EPOCH: f64 = 25569.0;

/// How a column is typed for charts and the workbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Quantitative,
    Temporal,
    Nominal,
}

impl Kind {
    /// The Vega-Lite `type` for a field of this kind.
    pub fn vega_type(self) -> &'static str {
        match self {
            Kind::Quantitative => "quantitative",
            Kind::Temporal => "temporal",
            Kind::Nominal => "nominal",
        }
    }

    fn of(data_type: &DataType) -> Kind {
        match data_type {
            DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, _) => Kind::Temporal,
            dt if dt.is_numeric() => Kind::Quantitative,
            _ => Kind::Nominal,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub kind: Kind,
}

/// One value. `Time` keeps the display text and the Excel date serial.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Null,
    Number(f64),
    Bool(bool),
    Text(String),
    Time { text: String, serial: f64 },
}

/// Rows kept for rendering, plus what was dropped to respect the row cap.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    /// Rows in the query result before capping.
    pub total_rows: usize,
}

/// How rows are chosen when the result exceeds the cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sampling {
    /// Keep the first `cap` rows (aggregated charts are already ordered by the query).
    Head,
    /// Keep `cap` rows spread evenly over the result (raw points such as scatter plots).
    Stride,
}

impl Table {
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.name == name)
    }

    /// Number of rows dropped by the cap.
    pub fn dropped_rows(&self) -> usize {
        self.total_rows - self.rows.len()
    }

    /// Builds a table from result batches, keeping at most `cap` rows.
    pub fn from_batches(
        batches: &[RecordBatch],
        cap: usize,
        sampling: Sampling,
    ) -> Result<Table, RenderError> {
        let total_rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
        let columns = match batches.first() {
            Some(first) => first
                .schema()
                .fields()
                .iter()
                .map(|f| Column {
                    name: f.name().clone(),
                    kind: Kind::of(f.data_type()),
                })
                .collect(),
            None => Vec::new(),
        };

        // Global row indices to keep, ascending.
        let wanted: Vec<usize> = if total_rows <= cap {
            (0..total_rows).collect()
        } else {
            match sampling {
                Sampling::Head => (0..cap).collect(),
                // Index i*total/cap is strictly increasing because total > cap.
                Sampling::Stride => (0..cap).map(|i| i * total_rows / cap).collect(),
            }
        };

        let mut rows = Vec::with_capacity(wanted.len());
        let mut wanted = wanted.into_iter().peekable();
        let mut offset = 0usize;
        for batch in batches {
            let n = batch.num_rows();
            if wanted.peek().is_some_and(|&i| i < offset + n) {
                let prepared = prepare_batch(batch)?;
                while let Some(&index) = wanted.peek() {
                    if index >= offset + n {
                        break;
                    }
                    wanted.next();
                    let local = index - offset;
                    rows.push(
                        prepared
                            .iter()
                            .map(|c| c.cell(local))
                            .collect::<Result<_, _>>()?,
                    );
                }
            }
            offset += n;
        }

        Ok(Table {
            columns,
            rows,
            total_rows,
        })
    }
}

/// A column of one batch, pre-converted so per-row access is cheap.
struct PreparedColumn {
    kind: Kind,
    source: ArrayRef,
    /// Float64 view for quantitative columns.
    floats: Option<ArrayRef>,
    /// Raw Int64 view (days, millis or timestamp units) for temporal columns.
    raw: Option<ArrayRef>,
    data_type: DataType,
}

fn prepare_batch(batch: &RecordBatch) -> Result<Vec<PreparedColumn>, RenderError> {
    let mut out = Vec::with_capacity(batch.num_columns());
    for (index, field) in batch.schema().fields().iter().enumerate() {
        let source = batch.column(index).clone();
        let kind = Kind::of(field.data_type());
        let floats = match kind {
            Kind::Quantitative => Some(cast(&source, &DataType::Float64).map_err(arrow_error)?),
            _ => None,
        };
        let raw = match kind {
            Kind::Temporal => Some(cast(&source, &DataType::Int64).map_err(arrow_error)?),
            _ => None,
        };
        out.push(PreparedColumn {
            kind,
            source,
            floats,
            raw,
            data_type: field.data_type().clone(),
        });
    }
    Ok(out)
}

impl PreparedColumn {
    fn cell(&self, row: usize) -> Result<Cell, RenderError> {
        if self.source.is_null(row) {
            return Ok(Cell::Null);
        }
        match self.kind {
            Kind::Quantitative => {
                let floats = self.floats.as_ref().expect("prepared");
                let value = floats.as_primitive::<Float64Type>().value(row);
                Ok(if value.is_finite() {
                    Cell::Number(value)
                } else {
                    Cell::Null
                })
            }
            Kind::Temporal => {
                let raw = self.raw.as_ref().expect("prepared");
                let value = raw.as_primitive::<Int64Type>().value(row);
                let text = array_value_to_string(&self.source, row).map_err(arrow_error)?;
                Ok(Cell::Time {
                    text,
                    serial: excel_serial(&self.data_type, value),
                })
            }
            Kind::Nominal => match &self.data_type {
                DataType::Boolean => Ok(Cell::Bool(self.source.as_boolean().value(row))),
                _ => Ok(Cell::Text(
                    array_value_to_string(&self.source, row).map_err(arrow_error)?,
                )),
            },
        }
    }
}

/// Converts a raw temporal value to an Excel date serial.
pub(crate) fn excel_serial(data_type: &DataType, value: i64) -> f64 {
    let days = match data_type {
        DataType::Date32 => value as f64,
        DataType::Date64 => value as f64 / 86_400_000.0,
        DataType::Timestamp(unit, _) => {
            let per_second = match unit {
                TimeUnit::Second => 1.0,
                TimeUnit::Millisecond => 1e3,
                TimeUnit::Microsecond => 1e6,
                TimeUnit::Nanosecond => 1e9,
            };
            value as f64 / per_second / 86_400.0
        }
        _ => f64::NAN,
    };
    days + EXCEL_UNIX_EPOCH
}

fn arrow_error(err: impl std::fmt::Display) -> RenderError {
    RenderError::Output(format!("cannot convert query result: {err}"))
}
