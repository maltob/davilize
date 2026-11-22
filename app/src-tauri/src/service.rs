//! Plain functions behind the Tauri commands. They take an [`Engine`] or a
//! [`Report`] and return serializable values, so they are tested without a window.
//! Chart output comes from `dv-render`, the same code the CLI uses, so the GUI
//! preview and an exported report agree.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use duckdb::arrow::array::Array;
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::display::array_value_to_string;
use dv_core::{Dataset, Report, Source, SourceKind, Visual};
use dv_engine::sql::quote_ident;
use dv_engine::{ColumnInfo, Engine, OutputFormat, compile_visual, infer_format};
use dv_render::Rendered;
use dv_render::html::Scripts;
use serde::Serialize;
use serde_json::Value;

use crate::error::ApiError;

/// Most rows a preview returns. The grid never receives a full table.
pub const MAX_PREVIEW_ROWS: u32 = 1_000;

/// A column as the field list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnView {
    pub name: String,
    pub data_type: String,
    /// `number`, `text`, `date`, `bool` or `other`. Drives the type icon.
    pub family: &'static str,
    /// `measure` for numbers (summed by default), `dimension` otherwise.
    pub role: &'static str,
}

/// A dataset (a view over a source) and its columns.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DatasetSchema {
    pub name: String,
    pub columns: Vec<ColumnView>,
}

/// Result of adding files: the new source and dataset, ready to append to a report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AddedDataset {
    pub source: Source,
    pub dataset: Dataset,
    pub columns: Vec<ColumnView>,
}

/// A report read from disk, with the schema of each of its datasets.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenedReport {
    pub report: Report,
    pub path: String,
    pub datasets: Vec<DatasetSchema>,
}

/// A header and cells as text, for the preview grid and the profile.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QueryTable {
    pub columns: Vec<ColumnHead>,
    /// `None` is SQL NULL.
    pub rows: Vec<Vec<Option<String>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnHead {
    pub name: String,
    pub data_type: String,
}

/// One visual's query, compiled SQL and Vega-Lite spec (data embedded).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RenderedVisual {
    pub id: String,
    pub title: String,
    pub spec: Value,
    pub notes: Vec<String>,
    pub sql: String,
    pub row_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Html,
    Png,
    Pdf,
    Xlsx,
    Csv,
}

impl ExportFormat {
    pub fn parse(text: &str) -> Result<Self, ApiError> {
        match text {
            "html" => Ok(ExportFormat::Html),
            "png" => Ok(ExportFormat::Png),
            "pdf" => Ok(ExportFormat::Pdf),
            "xlsx" => Ok(ExportFormat::Xlsx),
            "csv" => Ok(ExportFormat::Csv),
            other => Err(ApiError::invalid(format!(
                "unknown export format '{other}'"
            ))),
        }
    }
}

/// What an export wrote, and what it skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportSummary {
    pub files: Vec<String>,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Column typing and names
// ---------------------------------------------------------------------------

/// Maps a DuckDB type name to the family used for icons and default roles.
pub fn family(data_type: &str) -> &'static str {
    let base = data_type
        .split('(')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_uppercase();
    match base.as_str() {
        "TINYINT" | "SMALLINT" | "INTEGER" | "INT" | "INT4" | "BIGINT" | "INT8" | "HUGEINT"
        | "UTINYINT" | "USMALLINT" | "UINTEGER" | "UBIGINT" | "UHUGEINT" | "FLOAT" | "REAL"
        | "DOUBLE" | "DECIMAL" | "NUMERIC" => "number",
        "BOOLEAN" | "BOOL" => "bool",
        "VARCHAR" | "TEXT" | "STRING" | "CHAR" | "BPCHAR" | "UUID" | "ENUM" => "text",
        other if other.starts_with("DATE") || other.starts_with("TIME") => "date",
        _ => "other",
    }
}

/// Builds field-list entries from DuckDB's `DESCRIBE` output.
pub fn column_views(columns: Vec<ColumnInfo>) -> Vec<ColumnView> {
    columns
        .into_iter()
        .map(|c| {
            let family = family(&c.data_type);
            ColumnView {
                role: if family == "number" {
                    "measure"
                } else {
                    "dimension"
                },
                family,
                name: c.name,
                data_type: c.data_type,
            }
        })
        .collect()
}

/// Turns a file name stem into a valid relation name (letters, digits, `_`).
pub fn relation_base(stem: &str) -> String {
    let name: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.chars().all(|c| c == '_') {
        "data".to_string()
    } else {
        name
    }
}

/// `base`, or `base_2`, `base_3`, ... The dataset name and its `<name>_source` must both be free.
pub fn unique_relation(base: &str, taken: &HashSet<String>) -> String {
    let free = |name: &str| !taken.contains(name) && !taken.contains(&format!("{name}_source"));
    if free(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|candidate| free(candidate))
        .expect("an unused name exists")
}

/// A file name safe for output files, derived from a report name.
pub fn output_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = stem.trim_matches('_');
    if trimmed.is_empty() {
        "report".to_string()
    } else {
        trimmed.to_string()
    }
}

// ---------------------------------------------------------------------------
// Data sources
// ---------------------------------------------------------------------------

/// Registers data files as a source and a dataset each. `taken` holds the
/// relation names already in the report; new names avoid them.
pub fn add_data_files(
    engine: &Engine,
    taken: &[String],
    paths: &[PathBuf],
) -> Result<Vec<AddedDataset>, ApiError> {
    let mut taken: HashSet<String> = taken.iter().cloned().collect();
    let mut added = Vec::new();
    for path in paths {
        if !path.is_file() {
            return Err(ApiError::new(
                "io",
                format!("file not found: {}", path.display()),
            ));
        }
        let format = infer_format(path)?;
        let base = relation_base(path.file_stem().and_then(|s| s.to_str()).unwrap_or("data"));
        let name = unique_relation(&base, &taken);
        let source_name = format!("{name}_source");
        taken.insert(name.clone());
        taken.insert(source_name.clone());

        let source = Source {
            name: source_name,
            kind: SourceKind::File {
                path: path.clone(),
                format,
            },
        };
        engine.register_source(&source, Path::new(""))?;
        let dataset = Dataset {
            name: name.clone(),
            source: source.name.clone(),
            sql: None,
            calculated_fields: Vec::new(),
        };
        engine.register_dataset(&dataset)?;
        let columns = column_views(engine.schema(&name)?);
        added.push(AddedDataset {
            source,
            dataset,
            columns,
        });
    }
    Ok(added)
}

/// Opens a `.dvz` file in a fresh engine. Relative source paths resolve against the report's folder.
pub fn open_report(path: &Path) -> Result<(Engine, OpenedReport), ApiError> {
    let mut report = Report::load(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    // Resolve relative source paths now so a later "Save as" into another folder
    // re-relativises them correctly instead of leaving paths that point nowhere.
    for source in &mut report.sources {
        if let SourceKind::File { path: file, .. } = &mut source.kind
            && file.is_relative()
        {
            *file = base.join(&*file);
        }
    }
    let engine = Engine::new()?;
    engine.register_report(&report, base)?;
    let mut datasets = Vec::new();
    for dataset in &report.datasets {
        datasets.push(DatasetSchema {
            name: dataset.name.clone(),
            columns: column_views(engine.schema(&dataset.name)?),
        });
    }
    let opened = OpenedReport {
        report,
        path: path.display().to_string(),
        datasets,
    };
    Ok((engine, opened))
}

/// Validates and writes a report. Source files inside the report's folder are
/// stored relative to it, so the folder can be moved. Returns the stored report.
pub fn save_report(path: &Path, report: &Report) -> Result<Report, ApiError> {
    report.validate()?;
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let mut stored = report.clone();
    for source in &mut stored.sources {
        if let SourceKind::File { path: file, .. } = &mut source.kind
            && file.is_absolute()
            && let Ok(relative) = file.strip_prefix(dir)
            && !relative.as_os_str().is_empty()
        {
            *file = relative.to_path_buf();
        }
    }
    stored.save(path)?;
    Ok(stored)
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/// The first `limit` rows of a dataset (capped at [`MAX_PREVIEW_ROWS`]).
pub fn preview(engine: &Engine, dataset: &str, limit: u32) -> Result<QueryTable, ApiError> {
    let limit = limit.clamp(1, MAX_PREVIEW_ROWS);
    let columns = engine
        .schema(dataset)?
        .into_iter()
        .map(|c| ColumnHead {
            name: c.name,
            data_type: c.data_type,
        })
        .collect();
    let sql = format!("SELECT * FROM {} LIMIT {limit}", quote_ident(dataset));
    let batches = engine.query(&sql)?;
    table_from_batches(columns, &batches)
}

/// DuckDB's `SUMMARIZE` for a dataset: one row per column with its statistics.
pub fn profile(engine: &Engine, dataset: &str) -> Result<QueryTable, ApiError> {
    let batches = engine.profile(dataset)?;
    let columns = match batches.first() {
        Some(batch) => batch
            .schema()
            .fields()
            .iter()
            .map(|f| ColumnHead {
                name: f.name().clone(),
                data_type: f.data_type().to_string(),
            })
            .collect(),
        None => Vec::new(),
    };
    table_from_batches(columns, &batches)
}

fn table_from_batches(
    columns: Vec<ColumnHead>,
    batches: &[RecordBatch],
) -> Result<QueryTable, ApiError> {
    let mut rows = Vec::new();
    for batch in batches {
        for row in 0..batch.num_rows() {
            let mut cells = Vec::with_capacity(batch.num_columns());
            for column in batch.columns() {
                cells.push(if column.is_null(row) {
                    None
                } else {
                    Some(
                        array_value_to_string(column, row)
                            .map_err(|e| ApiError::internal(e.to_string()))?,
                    )
                });
            }
            rows.push(cells);
        }
    }
    Ok(QueryTable { columns, rows })
}

/// Compiles and runs one visual through the engine, then builds its Vega-Lite spec with `dv-render`.
pub fn render_visual(engine: &Engine, visual: &Visual) -> Result<RenderedVisual, ApiError> {
    let sql = compile_visual(visual)?;
    let batches = engine.query(&sql)?;
    let rendered = dv_render::prepare(visual, &batches)?;
    Ok(RenderedVisual {
        id: rendered.id.clone(),
        title: rendered.title.clone(),
        row_count: rendered.table.rows.len(),
        notes: rendered.notes.clone(),
        spec: rendered.spec.clone(),
        sql,
    })
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn render_one(engine: &Engine, visual: &Visual) -> Result<Rendered, ApiError> {
    let batches = engine.run_visual(visual)?;
    Ok(dv_render::prepare(visual, &batches)?)
}

fn render_all(engine: &Engine, report: &Report) -> Result<Vec<Rendered>, ApiError> {
    report
        .visuals
        .iter()
        .map(|visual| render_one(engine, visual))
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    std::fs::write(path, bytes)
        .map_err(|e| ApiError::new("io", format!("cannot write {}: {e}", path.display())))
}

/// Writes the report's visuals in `format` into `dir`. Every format handles every
/// chart type; `warnings` is kept for problems that do not stop the export.
///
/// * `html`: one page, `<report>.html`, with Vega-Lite loaded from the CDN.
/// * `xlsx`: one workbook, `<report>.xlsx`, with one sheet per visual.
/// * `csv`: one file per visual, `<visual id>.csv`, holding the visual's query result.
/// * `png` / `pdf`: one file per visual (native renderer, all chart types).
pub fn export_report(
    engine: &Engine,
    report: &Report,
    format: ExportFormat,
    dir: &Path,
) -> Result<ExportSummary, ApiError> {
    // Visual ids become file names, so the report must pass validation first.
    report.validate()?;
    std::fs::create_dir_all(dir)
        .map_err(|e| ApiError::new("io", format!("cannot create {}: {e}", dir.display())))?;
    let stem = output_stem(&report.name);
    let mut files = Vec::new();
    let warnings = Vec::new();

    match format {
        ExportFormat::Csv => {
            for visual in &report.visuals {
                let sql = compile_visual(visual)?;
                let path = dir.join(format!("{}.csv", visual.id));
                engine.export(&sql, &path, OutputFormat::Csv)?;
                files.push(path.display().to_string());
            }
        }
        ExportFormat::Png | ExportFormat::Pdf => {
            let extension = if format == ExportFormat::Png {
                "png"
            } else {
                "pdf"
            };
            for visual in &report.visuals {
                let rendered = render_one(engine, visual)?;
                let svg = dv_render::svg::render(&rendered)?;
                let bytes = if format == ExportFormat::Png {
                    dv_render::raster::png(&svg)?
                } else {
                    dv_render::raster::pdf(&svg)?
                };
                let path = dir.join(format!("{}.{extension}", visual.id));
                write_file(&path, &bytes)?;
                files.push(path.display().to_string());
            }
        }
        ExportFormat::Html => {
            let rendered = render_all(engine, report)?;
            let page = dv_render::html_report(&report.name, &rendered, &Scripts::Cdn)?;
            let path = dir.join(format!("{stem}.html"));
            write_file(&path, page.as_bytes())?;
            files.push(path.display().to_string());
        }
        ExportFormat::Xlsx => {
            let rendered = render_all(engine, report)?;
            let bytes = dv_render::xlsx::workbook(&rendered)?;
            let path = dir.join(format!("{stem}.xlsx"));
            write_file(&path, &bytes)?;
            files.push(path.display().to_string());
        }
    }
    Ok(ExportSummary { files, warnings })
}

#[cfg(test)]
mod tests;
