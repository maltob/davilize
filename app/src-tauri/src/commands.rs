//! Tauri commands. Each one hands its work to a blocking thread, so a slow
//! query never stalls the webview. Logic lives in [`crate::service`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dv_core::{Report, Visual};
use tauri::State;

use crate::error::ApiError;
use crate::service::{
    self, AddedDataset, ExportFormat, ExportSummary, OpenedReport, QueryTable, RenderedVisual,
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

/// Runs one visual and returns its Vega-Lite spec (built by dv-render).
#[tauri::command]
pub async fn render_visual(state: Shared<'_>, visual: Visual) -> Result<RenderedVisual, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let engine = state.lock_engine()?;
        service::render_visual(&engine, &visual)
    })
    .await
}

/// Writes every visual of `report` in `format` (html, png, pdf, xlsx, csv) into `dir`.
#[tauri::command]
pub async fn export_report(
    state: Shared<'_>,
    report: Report,
    format: String,
    dir: String,
) -> Result<ExportSummary, ApiError> {
    let state = state.inner().clone();
    blocking(move || {
        let format = ExportFormat::parse(&format)?;
        let engine = state.lock_engine()?;
        service::export_report(&engine, &report, format, Path::new(&dir))
    })
    .await
}

/// Interrupts the query that is running, if any. Does not wait for the engine.
#[tauri::command]
pub fn cancel_query(state: Shared<'_>) -> Result<(), ApiError> {
    state.cancel()
}
