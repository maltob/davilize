//! Plain functions behind the Tauri commands. They take an [`Engine`] or a
//! [`Report`] and return serializable values, so they are tested without a window.
//! Chart output comes from `dv-render`, the same code the CLI uses, so the GUI
//! preview and an exported report agree.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use duckdb::arrow::array::Array;
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::display::array_value_to_string;
use dv_core::{
    Dashboard, Dataset, Filter, ParamValues, Parameter, Report, Source, SourceKind, Visual,
};
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
    /// True for a column added by a calculated field.
    pub calculated: bool,
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
    /// Tableau workbook.
    Twb,
    /// Tableau packaged workbook (data files inside).
    Twbx,
    /// Power BI template.
    Pbit,
    /// Power BI Project folder.
    Pbip,
    /// Power BI Desktop file.
    Pbix,
}

impl ExportFormat {
    /// The interop format and output name for Tableau and Power BI exports.
    fn interop(self) -> Option<dv_interop::Format> {
        match self {
            ExportFormat::Twb => Some(dv_interop::Format::Twb),
            ExportFormat::Twbx => Some(dv_interop::Format::Twbx),
            ExportFormat::Pbit => Some(dv_interop::Format::Pbit),
            ExportFormat::Pbip => Some(dv_interop::Format::Pbip),
            ExportFormat::Pbix => Some(dv_interop::Format::Pbix),
            _ => None,
        }
    }

    pub fn parse(text: &str) -> Result<Self, ApiError> {
        match text {
            "twb" => Ok(ExportFormat::Twb),
            "twbx" => Ok(ExportFormat::Twbx),
            "pbit" => Ok(ExportFormat::Pbit),
            "pbip" => Ok(ExportFormat::Pbip),
            "pbix" => Ok(ExportFormat::Pbix),
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
                calculated: c.calculated,
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
            joins: Vec::new(),
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
// Sample report
// ---------------------------------------------------------------------------

/// Folder name of the sample report inside the system temp folder.
pub const SAMPLE_FOLDER: &str = "davilize-sample";

/// The bundled sample data (the same file as `examples/sample.csv`).
const SAMPLE_CSV: &str = include_str!("../../../examples/sample.csv");

/// The bundled sample report: one file source, one dataset, two charts and a
/// dashboard that places both. Relative source paths resolve to the report's folder.
const SAMPLE_REPORT: &str = r##"{
  "version": 1,
  "name": "Sample sales",
  "sources": [
    { "name": "sales_csv", "type": "file", "path": "sample.csv", "format": "csv" }
  ],
  "datasets": [
    { "name": "sales", "source": "sales_csv" }
  ],
  "visuals": [
    {
      "id": "revenue_by_region",
      "title": "Revenue by region",
      "dataset": "sales",
      "chart": "bar",
      "encodings": { "x": "region", "y": "total_revenue" },
      "dimensions": ["region"],
      "measures": [{ "field": "revenue", "agg": "sum", "alias": "total_revenue" }],
      "sort": { "by": "total_revenue", "direction": "desc" }
    },
    {
      "id": "revenue_by_product",
      "title": "Revenue by product",
      "dataset": "sales",
      "chart": "table",
      "dimensions": ["product"],
      "measures": [
        { "field": "revenue", "agg": "sum", "alias": "revenue_total" },
        { "field": "order_id", "agg": "count", "alias": "orders" }
      ],
      "sort": { "by": "revenue_total", "direction": "desc" }
    }
  ],
  "dashboards": [
    {
      "id": "overview",
      "title": "Overview",
      "columns": 12,
      "row_height": 80,
      "items": [
        { "visual": "revenue_by_region", "x": 0, "y": 0, "w": 6, "h": 4 },
        { "visual": "revenue_by_product", "x": 6, "y": 0, "w": 6, "h": 4 }
      ],
      "filters": []
    }
  ]
}
"##;

/// Writes the bundled sample data and report into `folder`, creating it if needed
/// and overwriting earlier copies. Returns the path of the `.dvz` file.
pub fn write_sample_report(folder: &Path) -> Result<PathBuf, ApiError> {
    std::fs::create_dir_all(folder)
        .map_err(|e| ApiError::new("io", format!("cannot create {}: {e}", folder.display())))?;
    let report = folder.join("sample.dvz");
    write_file(&folder.join("sample.csv"), SAMPLE_CSV.as_bytes())?;
    write_file(&report, SAMPLE_REPORT.as_bytes())?;
    Ok(report)
}

// ---------------------------------------------------------------------------
// Calculated fields
// ---------------------------------------------------------------------------

/// Checks one calculated field against the report's sources and the fields
/// defined before it in its dataset. Runs on a scratch engine, so the open
/// report is untouched. Errors come from the engine and name the field.
pub fn validate_calculated_field(
    report: &Report,
    base_dir: &Path,
    dataset: &str,
    field: &str,
) -> Result<(), ApiError> {
    let target = report
        .datasets
        .iter()
        .find(|d| d.name == dataset)
        .ok_or_else(|| ApiError::invalid(format!("unknown dataset '{dataset}'")))?;
    let position = target
        .calculated_fields
        .iter()
        .position(|f| f.name == field)
        .ok_or_else(|| {
            ApiError::invalid(format!(
                "calculated field '{field}' is not defined on dataset '{dataset}'"
            ))
        })?;

    let engine = Engine::new()?;
    // The dataset's own source must register, or its error is the useful one.
    let own_source = report
        .sources
        .iter()
        .find(|s| s.name == target.source)
        .ok_or_else(|| {
            ApiError::invalid(format!(
                "dataset '{dataset}' references unknown source '{}'",
                target.source
            ))
        })?;
    engine.register_source(own_source, base_dir)?;
    // Other sources and datasets are best effort: a dataset SQL may use them, and
    // a broken one elsewhere must not hide this field's result.
    for source in &report.sources {
        if source.name != own_source.name {
            let _ = engine.register_source(source, base_dir);
        }
    }
    for other in &report.datasets {
        if other.name != dataset {
            let _ = engine.register_dataset(other);
        }
    }

    // Only the fields up to and including this one, so later fields cannot affect it.
    let mut scoped = target.clone();
    scoped.calculated_fields.truncate(position + 1);
    engine.register_dataset(&scoped)?;
    Ok(())
}

/// Re-registers a dataset in the open engine after its calculated fields were
/// edited, and returns its refreshed columns. The view is replaced only when
/// every field checks, so a bad edit leaves the previous view in place.
pub fn register_dataset_fields(
    engine: &Engine,
    dataset: &Dataset,
) -> Result<Vec<ColumnView>, ApiError> {
    engine.register_dataset(dataset)?;
    Ok(column_views(engine.schema(&dataset.name)?))
}

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

/// Parses the user's parameter text per declared type and fills in defaults.
/// Bad or missing values come back as `invalid` errors that name the parameter.
pub fn bind_params(
    parameters: &[Parameter],
    values: &BTreeMap<String, String>,
) -> Result<ParamValues, ApiError> {
    let supplied: Vec<(String, String)> = values
        .iter()
        .map(|(name, text)| (name.clone(), text.clone()))
        .collect();
    Ok(dv_core::resolve_parameters(parameters, &supplied)?)
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
/// `values` are the user's parameter text; parameters without a value use their default.
pub fn render_visual(
    engine: &Engine,
    visual: &Visual,
    parameters: &[Parameter],
    values: &BTreeMap<String, String>,
) -> Result<RenderedVisual, ApiError> {
    let bound = dv_core::bind_visual(visual, &bind_params(parameters, values)?)?;
    let sql = compile_visual(&bound)?;
    let batches = engine.query(&sql)?;
    let rendered = dv_render::prepare(&bound, &batches)?;
    Ok(to_rendered_visual(&rendered, sql))
}

/// Runs one visual as it appears on a dashboard: the dashboard's shared `filters`
/// are added (only when the visual's dataset has the filtered column), then the
/// result is rendered as [`render_visual`] does. Parameters are bound from `values`.
pub fn render_dashboard_visual(
    engine: &Engine,
    visual: &Visual,
    filters: &[Filter],
    parameters: &[Parameter],
    values: &BTreeMap<String, String>,
) -> Result<RenderedVisual, ApiError> {
    let params = bind_params(parameters, values)?;
    let bound = dv_core::bind_visual(visual, &params)?;
    let columns = dataset_columns(engine, &visual.dataset)?;
    let mut dashboard = Dashboard::new("preview", "Preview");
    dashboard.filters = filters.to_vec();
    let shared = dv_core::apply_dashboard_filters(&dashboard, &bound, &columns, &params)?;
    let sql = engine.compile(&shared)?;
    let batches = engine.query(&sql)?;
    let rendered = dv_render::prepare(&shared, &batches)?;
    Ok(to_rendered_visual(&rendered, sql))
}

fn to_rendered_visual(rendered: &Rendered, sql: String) -> RenderedVisual {
    RenderedVisual {
        id: rendered.id.clone(),
        title: rendered.title.clone(),
        row_count: rendered.table.rows.len(),
        notes: rendered.notes.clone(),
        spec: rendered.spec.clone(),
        sql,
    }
}

/// Column names of a dataset, as the engine sees them (calculated fields included).
fn dataset_columns(engine: &Engine, dataset: &str) -> Result<Vec<String>, ApiError> {
    Ok(engine
        .schema(dataset)?
        .into_iter()
        .map(|c| c.name)
        .collect())
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn render_one(
    engine: &Engine,
    visual: &Visual,
    values: &ParamValues,
) -> Result<Rendered, ApiError> {
    let bound = dv_core::bind_visual(visual, values)?;
    let batches = engine.run_visual(&bound)?;
    Ok(dv_render::prepare(&bound, &batches)?)
}

fn render_all(
    engine: &Engine,
    report: &Report,
    values: &ParamValues,
) -> Result<Vec<Rendered>, ApiError> {
    report
        .visuals
        .iter()
        .map(|visual| render_one(engine, visual, values))
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
///
/// `values` are the user's parameter text, bound against `report.parameters` the same
/// way as a preview. Parameters without a value use their default.
pub fn export_report(
    engine: &Engine,
    report: &Report,
    format: ExportFormat,
    dir: &Path,
    values: &BTreeMap<String, String>,
) -> Result<ExportSummary, ApiError> {
    // Visual ids become file names, so the report must pass validation first.
    report.validate()?;
    let bound = bind_params(&report.parameters, values)?;
    std::fs::create_dir_all(dir)
        .map_err(|e| ApiError::new("io", format!("cannot create {}: {e}", dir.display())))?;
    let stem = output_stem(&report.name);
    if let Some(interop) = format.interop() {
        return export_interop(report, interop, dir, &stem);
    }
    let mut files = Vec::new();
    let warnings = Vec::new();

    match format {
        ExportFormat::Twb
        | ExportFormat::Twbx
        | ExportFormat::Pbit
        | ExportFormat::Pbip
        | ExportFormat::Pbix => {
            unreachable!("handled above")
        }
        ExportFormat::Csv => {
            for visual in &report.visuals {
                let sql = compile_visual(&dv_core::bind_visual(visual, &bound)?)?;
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
                let rendered = render_one(engine, visual, &bound)?;
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
            // With dashboards, the page is one grid per dashboard (as the CLI writes it).
            let page = if report.dashboards.is_empty() {
                let rendered = render_all(engine, report, &bound)?;
                dv_render::html_report(&report.name, &rendered, &Scripts::Cdn)?
            } else {
                let views = dv_render::dashboard_views(
                    report,
                    &bound,
                    |dataset| dataset_columns(engine, dataset),
                    |visual| {
                        let batches = engine.run_visual(visual)?;
                        Ok(dv_render::prepare(visual, &batches)?)
                    },
                    ApiError::from,
                )?;
                dv_render::html_dashboards(&report.name, &views, &Scripts::Cdn)?
            };
            let path = dir.join(format!("{stem}.html"));
            write_file(&path, page.as_bytes())?;
            files.push(path.display().to_string());
        }
        ExportFormat::Xlsx => {
            let rendered = render_all(engine, report, &bound)?;
            let bytes = dv_render::xlsx::workbook(&rendered)?;
            let path = dir.join(format!("{stem}.xlsx"));
            write_file(&path, &bytes)?;
            files.push(path.display().to_string());
        }
    }
    Ok(ExportSummary { files, warnings })
}

fn interop_error(err: dv_interop::InteropError) -> ApiError {
    match err {
        dv_interop::InteropError::Io { .. } => ApiError::new("io", err.to_string()),
        other => ApiError::invalid(other.to_string()),
    }
}

/// Writes the report as a Tableau or Power BI file into `dir`. The report's source
/// paths are already absolute in the app, so no base folder is needed.
fn export_interop(
    report: &Report,
    format: dv_interop::Format,
    dir: &Path,
    stem: &str,
) -> Result<ExportSummary, ApiError> {
    use dv_interop::Format;
    let out = match format {
        Format::Twb => dir.join(format!("{stem}.twb")),
        Format::Twbx => dir.join(format!("{stem}.twbx")),
        Format::Pbit => dir.join(format!("{stem}.pbit")),
        Format::Pbix => dir.join(format!("{stem}.pbix")),
        _ => dir.join(format!("{stem}_pbip")),
    };
    let exported = if format == Format::Pbix {
        // A .pbix stores calculated columns with their data: the engine computes it.
        let engine = Engine::new()?;
        engine.register_report(report, Path::new(""))?;
        let compute = |dataset: &str, column: &str| {
            engine
                .column_text(dataset, column)
                .map_err(|e| e.to_string())
        };
        let describe = |dataset: &str| {
            engine
                .schema(dataset)
                .map(|columns| {
                    columns
                        .into_iter()
                        .map(|c| (c.name, dv_interop::model_type(&c.data_type).to_string()))
                        .collect()
                })
                .map_err(|e| e.to_string())
        };
        let options = dv_interop::ExportOptions {
            column_values: Some(&compute),
            dataset_columns: Some(&describe),
        };
        dv_interop::export_with(report, Path::new(""), format, &out, &options)
    } else {
        dv_interop::export(report, Path::new(""), format, &out)
    }
    .map_err(interop_error)?;
    Ok(ExportSummary {
        files: exported
            .files
            .iter()
            .map(|f| f.display().to_string())
            .collect(),
        warnings: exported.warnings,
    })
}

/// A Tableau or Power BI file converted to a `.dvz` report and opened.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ImportedReport {
    pub opened: OpenedReport,
    /// What could not be converted, one sentence each.
    pub warnings: Vec<String>,
}

/// A path for `<stem>.dvz` in `dir` that does not exist yet.
fn free_report_path(dir: &Path, stem: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.dvz"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).dvz")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

/// Converts a Tableau or Power BI file into a `.dvz` saved next to it (under a new name
/// if one exists), then opens that report. Data files inside a packaged workbook are
/// unpacked next to the report.
pub fn import_report(path: &Path) -> Result<(Engine, ImportedReport), ApiError> {
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let checker = Engine::new()?;
    let check = |field: &str, expression: &str| {
        checker
            .check_calculated_expression(field, expression)
            .map_err(|e| e.to_string())
    };
    let options = dv_interop::ImportOptions {
        check_expression: Some(&check),
    };
    let imported = dv_interop::import(path, &dir, &options).map_err(interop_error)?;
    let stem = path.file_stem().map_or_else(
        || "imported".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let target = free_report_path(&dir, &stem);
    imported.report.save(&target)?;
    let (engine, opened) = open_report(&target).map_err(|e| {
        ApiError::new(
            "invalid",
            format!(
                "The file was converted and saved as {}, but it cannot be opened yet: {}",
                target.display(),
                e.message
            ),
        )
    })?;
    Ok((
        engine,
        ImportedReport {
            opened,
            warnings: imported.warnings,
        },
    ))
}

#[cfg(test)]
mod tests;
