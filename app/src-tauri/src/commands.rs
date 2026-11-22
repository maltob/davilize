//! Tauri commands. Each one hands its work to a blocking thread, so a slow
//! query never stalls the webview. Logic lives in [`crate::service`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dv_core::{Dataset, Filter, Parameter, Report, Visual};
use tauri::State;

use crate::error::ApiError;
use crate::service::{
    self, AddedDataset, ColumnView, ExportFormat, ExportSummary, OpenedReport, QueryTable,
    RenderedVisual,
};
use crate::state::AppState;

type Shared<'a> = State<'a, Arc<AppState>>;

/// Runs `work` on a blocking thread and returns its result.
async fn blocking<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(result) => result,
        Err(err) => Err(ApiError::internal(format!("background task failed: {err}"))),
    }
}

/// Registers data files with the engine. `existing_names` are the report's current names.
#[tauri::command]
pub async fn add_data_files(
    state: Shared<'_>,
    existing_names: Vec<String>,
    paths: Vec<String>,
) -> Result<Vec<AddedDataset>, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
        let engine = state.lock_engine()?;
        service::add_data_files(&engine, &existing_names, &paths)
    })
    .await
}

/// Reads a `.dvz` file and replaces the engine's sources with the report's.
#[tauri::command]
pub async fn open_report(state: Shared<'_>, path: String) -> Result<OpenedReport, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let (engine, opened) = service::open_report(Path::new(&path))?;
        state.replace_engine(engine)?;
        Ok(opened)
    })
    .await
}

/// Validates and writes a `.dvz` file. Returns the stored report.
#[tauri::command]
pub async fn save_report(path: String, report: Report) -> Result<Report, ApiError> {
    blocking(move || service::save_report(Path::new(&path), &report)).await
}

/// Writes the bundled sample report into a `davilize-sample` folder in the user's
/// temp directory and returns the absolute path of its `.dvz` file.
#[tauri::command]
pub async fn create_sample_report() -> Result<String, ApiError> {
    blocking(|| {
        let folder = std::env::temp_dir().join(service::SAMPLE_FOLDER);
        let path = service::write_sample_report(&folder)?;
        Ok(path.display().to_string())
    })
    .await
}

/// The first rows of a dataset for the preview grid.
#[tauri::command]
pub async fn preview_dataset(
    state: Shared<'_>,
    dataset: String,
    limit: Option<u32>,
) -> Result<QueryTable, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::preview(
            &engine,
            &dataset,
            limit.unwrap_or(service::MAX_PREVIEW_ROWS),
        )
    })
    .await
}

/// Per-column statistics (DuckDB `SUMMARIZE`) for a dataset.
#[tauri::command]
pub async fn profile_dataset(state: Shared<'_>, dataset: String) -> Result<QueryTable, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::profile(&engine, &dataset)
    })
    .await
}

/// Runs one visual as placed on a dashboard: `filters` are the dashboard's shared
/// filters, added for the visual's dataset when it has the column. Same parameter
/// rules as `render_visual`.
#[tauri::command]
pub async fn render_dashboard_visual(
    state: Shared<'_>,
    visual: Visual,
    filters: Vec<Filter>,
    parameters: Vec<Parameter>,
    values: BTreeMap<String, String>,
) -> Result<RenderedVisual, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::render_dashboard_visual(&engine, &visual, &filters, &parameters, &values)
    })
    .await
}

/// Runs one visual and returns its Vega-Lite spec (built by dv-render).
/// `parameters` are the report's declarations and `values` the user's text per name
/// (`{}` uses every default).
#[tauri::command]
pub async fn render_visual(
    state: Shared<'_>,
    visual: Visual,
    parameters: Vec<Parameter>,
    values: BTreeMap<String, String>,
) -> Result<RenderedVisual, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::render_visual(&engine, &visual, &parameters, &values)
    })
    .await
}

/// Writes every visual of `report` in `format` (html, png, pdf, xlsx, csv) into `dir`.
/// The report's own `parameters` are bound with `values`, as in a preview.
#[tauri::command]
pub async fn export_report(
    state: Shared<'_>,
    report: Report,
    format: String,
    dir: String,
    values: BTreeMap<String, String>,
) -> Result<ExportSummary, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let format = ExportFormat::parse(&format)?;
        let engine = state.lock_engine()?;
        service::export_report(&engine, &report, format, Path::new(&dir), &values)
    })
    .await
}

/// Checks one calculated field of `dataset` without touching the open report.
/// `report` must hold the field as the user has it (including unsaved edits), and
/// `report_path` (the `.dvz` file, or null when unsaved) sets the base for relative
/// source paths. Returns `null` when the field is valid; otherwise an `invalid`
/// error whose message names the field.
#[tauri::command]
pub async fn validate_calculated_field(
    report: Report,
    report_path: Option<String>,
    dataset: String,
    field: String,
) -> Result<(), ApiError> {
    blocking(move || {
        let base = report_path
            .as_deref()
            .and_then(|p| Path::new(p).parent())
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        service::validate_calculated_field(&report, &base, &dataset, &field)
    })
    .await
}

/// Re-registers a dataset (with its current calculated fields) in the open engine,
/// so previews and charts see the edits before the report is saved. Returns the
/// dataset's refreshed columns.
#[tauri::command]
pub async fn register_calculated_fields(
    state: Shared<'_>,
    dataset: Dataset,
) -> Result<Vec<ColumnView>, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::register_dataset_fields(&engine, &dataset)
    })
    .await
}

/// Interrupts the query that is running, if any. Does not wait for the engine.
#[tauri::command]
pub fn cancel_query(state: Shared<'_>) -> Result<(), ApiError> {
    state.cancel()
}
