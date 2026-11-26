//! WebAssembly bindings: the pure parts of the Davilize engine for the browser.
//!
//! The browser runs queries in DuckDB-Wasm (the native DuckDB crate does not build for
//! WebAssembly). Everything else is the same Rust code as the CLI and the desktop app:
//! parameters, SQL compilation, joins, calculated-field checks, dashboard filters, chart
//! specs, and the HTML and XLSX writers. Each function takes and returns JSON text (or raw
//! bytes), so the page orchestrates the queries and these functions never block on them.
//!
//! Every function that returns text returns `{"ok": true, ...}` or
//! `{"ok": false, "error": "..."}`.

use std::collections::BTreeMap;
use std::io::Cursor;

mod sqlite;

use arrow::ipc::reader::StreamReader;
use arrow::record_batch::RecordBatch;
use dv_core::{
    CalculatedField, Dashboard, Dataset, Filter, ParamValues, Parameter, Report, Visual,
};
use dv_render::Rendered;
use dv_render::html::Scripts;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use wasm_bindgen::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn failure(message: impl ToString) -> String {
    json!({ "ok": false, "error": message.to_string() }).to_string()
}

fn success(mut body: Value) -> String {
    if let Value::Object(map) = &mut body {
        map.insert("ok".into(), Value::Bool(true));
    }
    body.to_string()
}

fn parse<T: DeserializeOwned>(what: &str, text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| format!("invalid {what}: {e}"))
}

/// Runs `body` and turns its error into the failure JSON.
fn respond(body: impl FnOnce() -> Result<Value, String>) -> String {
    match body() {
        Ok(value) => success(value),
        Err(error) => failure(error),
    }
}

/// Parameter values from the user's text, with defaults for the rest.
fn bind_values(parameters: &[Parameter], values_json: &str) -> Result<ParamValues, String> {
    let values: BTreeMap<String, String> = parse("parameter values", values_json)?;
    let supplied: Vec<(String, String)> = values.into_iter().collect();
    dv_core::resolve_parameters(parameters, &supplied).map_err(|e| e.to_string())
}

/// Reads an Arrow IPC stream (what `apache-arrow`'s `tableToIPC(table, "stream")` writes).
fn read_batches(ipc: &[u8]) -> Result<Vec<RecordBatch>, String> {
    let reader = StreamReader::try_new(Cursor::new(ipc), None)
        .map_err(|e| format!("cannot read the query result: {e}"))?;
    reader
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot read the query result: {e}"))
}

fn rendered_json(rendered: &Rendered) -> Value {
    json!({
        "id": rendered.id,
        "title": rendered.title,
        "spec": rendered.spec,
        "notes": rendered.notes,
        "row_count": rendered.table.rows.len(),
    })
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// The schema version this build reads.
#[wasm_bindgen]
pub fn report_version() -> u32 {
    dv_core::REPORT_VERSION
}

/// Parses and validates a `.dvz` report. On success returns `{"report": {...}}` (the report
/// as stored, with its defaults filled in).
#[wasm_bindgen]
pub fn open_report(text: &str) -> String {
    respond(|| {
        let report = Report::from_json(text).map_err(|e| e.to_string())?;
        Ok(json!({ "report": report }))
    })
}

/// Validates a report built in the editor and returns it as pretty JSON text:
/// `{"text": "..."}`.
#[wasm_bindgen]
pub fn save_report(report_json: &str) -> String {
    respond(|| {
        let report: Report = parse("report", report_json)?;
        report.validate().map_err(|e| e.to_string())?;
        let text = report.to_json().map_err(|e| e.to_string())?;
        Ok(json!({ "text": text }))
    })
}

// ---------------------------------------------------------------------------
// Datasets
// ---------------------------------------------------------------------------

/// Validates a dataset's joins and calculated fields (names, shapes), without running SQL.
#[wasm_bindgen]
pub fn check_dataset(dataset_json: &str) -> String {
    respond(|| {
        let dataset: Dataset = parse("dataset", dataset_json)?;
        dataset
            .validate_calculated_fields()
            .map_err(|e| e.to_string())?;
        dataset.validate_joins().map_err(|e| e.to_string())?;
        Ok(json!({}))
    })
}

/// The SELECT that defines a dataset before its calculated fields: the joins over the
/// source, the custom SQL, or the source itself.
///
/// `relation_columns_json` maps each relation the dataset reads (its source and every join
/// target) to its column names, which the page gets from `DESCRIBE`.
#[wasm_bindgen]
pub fn dataset_base_sql(dataset_json: &str, relation_columns_json: &str) -> String {
    respond(|| {
        let dataset: Dataset = parse("dataset", dataset_json)?;
        let columns: BTreeMap<String, Vec<String>> =
            parse("relation columns", relation_columns_json)?;
        dataset
            .validate_calculated_fields()
            .map_err(|e| e.to_string())?;
        dataset.validate_joins().map_err(|e| e.to_string())?;
        let sql = if !dataset.joins.is_empty() {
            let columns_of = |relation: &str| Ok(columns.get(relation).cloned());
            dv_sql::join_select(&dataset, &columns_of).map_err(|e| e.to_string())?
        } else {
            match &dataset.sql {
                Some(sql) => sql.clone(),
                None => format!("SELECT * FROM {}", dv_sql::quote_ident(&dataset.source)),
            }
        };
        Ok(json!({ "sql": sql }))
    })
}

/// Wraps a base SELECT in one layer per calculated field, in order. Returns the final SQL and
/// each layer (so the page can describe them one by one and name the field that fails).
/// `base_columns_json` is the base SELECT's column names.
#[wasm_bindgen]
pub fn dataset_layers(dataset_json: &str, base_sql: &str, base_columns_json: &str) -> String {
    respond(|| {
        let dataset: Dataset = parse("dataset", dataset_json)?;
        let base_columns: Vec<String> = parse("columns", base_columns_json)?;
        let mut taken: std::collections::HashSet<String> = base_columns
            .iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let mut body = base_sql.to_string();
        let mut layers = Vec::new();
        for CalculatedField {
            name, expression, ..
        } in &dataset.calculated_fields
        {
            let key = name.to_ascii_lowercase();
            if taken.contains(&key) {
                return Err(format!(
                    "invalid spec: calculated field '{name}' has the same name as a column of dataset '{}'",
                    dataset.name
                ));
            }
            body = format!(
                "SELECT *, ({expression}) AS {} FROM ({body})",
                dv_sql::quote_ident(name)
            );
            taken.insert(key);
            layers.push(json!({ "field": name, "sql": body }));
        }
        Ok(json!({ "sql": body, "layers": layers }))
    })
}

/// The query that makes DuckDB parse a calculated-field expression. Run it, then pass its
/// single text result to [`calc_verify`].
#[wasm_bindgen]
pub fn calc_parse_query(field: &str, expression: &str) -> String {
    respond(|| {
        dv_sql::calc::precheck(field, expression).map_err(|e| e.to_string())?;
        Ok(json!({ "sql": dv_sql::calc::parse_query(expression) }))
    })
}

/// Checks the parse tree of a calculated-field expression against the allowlist.
#[wasm_bindgen]
pub fn calc_verify(field: &str, serialized: &str) -> String {
    respond(|| {
        dv_sql::calc::verify(field, serialized).map_err(|e| e.to_string())?;
        Ok(json!({}))
    })
}

// ---------------------------------------------------------------------------
// Visuals
// ---------------------------------------------------------------------------

/// Binds parameters and compiles a visual to SQL. With `filters_json` (a dashboard's shared
/// filters, or `""`), the filters are added where the dataset has the field
/// (`columns_json` lists the dataset's columns).
///
/// Returns `{"sql": "...", "visual": {...}}` where `visual` is the bound, filtered visual to
/// pass to [`prepare_visual`].
#[wasm_bindgen]
pub fn compile_visual(
    visual_json: &str,
    parameters_json: &str,
    values_json: &str,
    filters_json: &str,
    columns_json: &str,
) -> String {
    respond(|| {
        let visual: Visual = parse("visual", visual_json)?;
        let parameters: Vec<Parameter> = parse("parameters", parameters_json)?;
        let values = bind_values(&parameters, values_json)?;
        let bound = dv_core::bind_visual(&visual, &values).map_err(|e| e.to_string())?;
        let columns: Vec<String> = parse("columns", columns_json)?;
        let shared = if filters_json.trim().is_empty() {
            bound
        } else {
            let filters: Vec<Filter> = parse("filters", filters_json)?;
            let mut dashboard = Dashboard::new("preview", "Preview");
            dashboard.filters = filters;
            dv_core::apply_dashboard_filters(&dashboard, &bound, &columns, &values)
                .map_err(|e| e.to_string())?
        };
        let columns_for_sql = if columns.is_empty() {
            None
        } else {
            Some(columns.as_slice())
        };
        let sql =
            dv_sql::compile_visual_with(&shared, columns_for_sql).map_err(|e| e.to_string())?;
        Ok(json!({ "sql": sql, "visual": shared }))
    })
}

/// Builds a visual's Vega-Lite spec from its query result (an Arrow IPC stream).
/// Returns `{"id", "title", "spec", "notes", "row_count"}`.
#[wasm_bindgen]
pub fn prepare_visual(visual_json: &str, ipc: &[u8]) -> String {
    respond(|| {
        let visual: Visual = parse("visual", visual_json)?;
        let batches = read_batches(ipc)?;
        let rendered = dv_render::prepare(&visual, &batches).map_err(|e| e.to_string())?;
        Ok(rendered_json(&rendered))
    })
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

/// The visuals an export runs, as `{"key", "visual", "sql"}` entries: one per visual of the
/// report, or (HTML of a report with dashboards) one per dashboard placement.
///
/// `columns_by_dataset_json` maps each dataset to its columns.
#[wasm_bindgen]
pub fn export_plan(
    report_json: &str,
    values_json: &str,
    columns_by_dataset_json: &str,
    by_dashboard: bool,
) -> String {
    respond(|| {
        let report: Report = parse("report", report_json)?;
        report.validate().map_err(|e| e.to_string())?;
        let values = bind_values(&report.parameters, values_json)?;
        let columns: BTreeMap<String, Vec<String>> = parse("columns", columns_by_dataset_json)?;
        let mut items = Vec::new();
        if by_dashboard && !report.dashboards.is_empty() {
            let planned = dv_render::dashboard_plan(
                &report,
                &values,
                |dataset| Ok::<_, String>(columns.get(dataset).cloned().unwrap_or_default()),
                |e| e.to_string(),
            )?;
            for dashboard in planned {
                for (index, (.., visual)) in dashboard.charts.iter().enumerate() {
                    let sql = dv_sql::compile_visual_with(
                        visual,
                        columns.get(&visual.dataset).map(Vec::as_slice),
                    )
                    .map_err(|e| e.to_string())?;
                    items.push(json!({
                        "key": format!("{}#{index}", dashboard.id),
                        "visual": visual,
                        "sql": sql,
                    }));
                }
            }
        } else {
            for visual in &report.visuals {
                let bound = dv_core::bind_visual(visual, &values).map_err(|e| e.to_string())?;
                let sql = dv_sql::compile_visual_with(
                    &bound,
                    columns.get(&bound.dataset).map(Vec::as_slice),
                )
                .map_err(|e| e.to_string())?;
                items.push(json!({ "key": visual.id, "visual": bound, "sql": sql }));
            }
        }
        Ok(json!({ "items": items }))
    })
}

/// Splits one buffer into the Arrow IPC streams the page concatenated, by `lengths`.
fn split_results(buffer: &[u8], lengths: &[usize]) -> Result<Vec<Vec<RecordBatch>>, String> {
    let mut offset = 0;
    let mut out = Vec::new();
    for length in lengths {
        let end = offset + length;
        let chunk = buffer
            .get(offset..end)
            .ok_or("query results are shorter than their lengths")?;
        out.push(read_batches(chunk)?);
        offset = end;
    }
    Ok(out)
}

fn render_all(
    visuals: &[Visual],
    buffer: &[u8],
    lengths_json: &str,
) -> Result<Vec<Rendered>, String> {
    let lengths: Vec<usize> = parse("lengths", lengths_json)?;
    if lengths.len() != visuals.len() {
        return Err("each visual needs a query result".into());
    }
    let results = split_results(buffer, &lengths)?;
    visuals
        .iter()
        .zip(&results)
        .map(|(visual, batches)| dv_render::prepare(visual, batches).map_err(|e| e.to_string()))
        .collect()
}

/// The HTML report page. `visuals_json` is the `visual` of each [`export_plan`] item, in
/// order; `results` is their Arrow IPC streams concatenated, with `lengths_json` their sizes.
/// For a report with dashboards, plan with `by_dashboard = true`.
#[wasm_bindgen]
pub fn export_html(
    report_json: &str,
    values_json: &str,
    columns_by_dataset_json: &str,
    results: &[u8],
    lengths_json: &str,
) -> String {
    respond(|| {
        let report: Report = parse("report", report_json)?;
        let values = bind_values(&report.parameters, values_json)?;
        let columns: BTreeMap<String, Vec<String>> = parse("columns", columns_by_dataset_json)?;
        let lengths: Vec<usize> = parse("lengths", lengths_json)?;
        let page = if report.dashboards.is_empty() {
            let visuals: Vec<Visual> = report
                .visuals
                .iter()
                .map(|v| dv_core::bind_visual(v, &values).map_err(|e| e.to_string()))
                .collect::<Result<_, _>>()?;
            let rendered = render_all(&visuals, results, lengths_json)?;
            dv_render::html_report(&report.name, &rendered, &Scripts::Cdn)
                .map_err(|e| e.to_string())?
        } else {
            let planned = dv_render::dashboard_plan(
                &report,
                &values,
                |dataset| Ok::<_, String>(columns.get(dataset).cloned().unwrap_or_default()),
                |e| e.to_string(),
            )?;
            let all: Vec<Visual> = planned
                .iter()
                .flat_map(|d| d.charts.iter().map(|(.., v)| v.clone()))
                .collect();
            if all.len() != lengths.len() {
                return Err("each placed visual needs a query result".into());
            }
            let mut rendered = render_all(&all, results, lengths_json)?.into_iter();
            let mut views = Vec::new();
            for dashboard in planned {
                let mut charts = Vec::new();
                for (x, y, w, h, _) in &dashboard.charts {
                    charts.push(dv_render::ChartPlacement {
                        x: *x,
                        y: *y,
                        w: *w,
                        h: *h,
                        chart: rendered.next().ok_or("missing rendered chart")?,
                    });
                }
                views.push(dv_render::DashboardView {
                    id: dashboard.id,
                    title: dashboard.title,
                    columns: dashboard.columns,
                    row_height: dashboard.row_height,
                    charts,
                });
            }
            dv_render::html_dashboards(&report.name, &views, &Scripts::Cdn)
                .map_err(|e| e.to_string())?
        };
        Ok(json!({ "html": page }))
    })
}

/// The XLSX workbook, one sheet per visual. Inputs as for [`export_html`] with
/// `by_dashboard = false`. Returns the workbook bytes, or an empty array on error (call
/// [`xlsx_error`] for the reason).
#[wasm_bindgen]
pub fn export_xlsx(visuals_json: &str, results: &[u8], lengths_json: &str) -> Vec<u8> {
    xlsx(visuals_json, results, lengths_json).unwrap_or_default()
}

/// Why [`export_xlsx`] returned nothing, or `""` when it works.
#[wasm_bindgen]
pub fn xlsx_error(visuals_json: &str, results: &[u8], lengths_json: &str) -> String {
    xlsx(visuals_json, results, lengths_json)
        .err()
        .unwrap_or_default()
}

fn xlsx(visuals_json: &str, results: &[u8], lengths_json: &str) -> Result<Vec<u8>, String> {
    let visuals: Vec<Visual> = parse("visuals", visuals_json)?;
    let rendered = render_all(&visuals, results, lengths_json)?;
    dv_render::xlsx::workbook(&rendered).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Tableau and Power BI
// ---------------------------------------------------------------------------
//
// The converters read and write files. A browser has none, so they use an in-memory store:
// the page puts the files an import reads into it (`vfs_put`), runs the conversion, and takes
// the files it wrote out (`vfs_take`).

/// Empties the in-memory file store.
#[wasm_bindgen]
pub fn vfs_clear() {
    dv_interop::fsx::memfs::clear();
}

/// Adds a file to the in-memory store.
#[wasm_bindgen]
pub fn vfs_put(path: &str, bytes: &[u8]) {
    dv_interop::fsx::memfs::put(std::path::Path::new(path), bytes.to_vec());
}

/// A file from the in-memory store, or nothing.
#[wasm_bindgen]
pub fn vfs_take(path: &str) -> Option<Vec<u8>> {
    dv_interop::fsx::memfs::take(std::path::Path::new(path))
}

/// Every path in the in-memory store, as a JSON array.
#[wasm_bindgen]
pub fn vfs_list() -> String {
    json!(dv_interop::fsx::memfs::list()).to_string()
}

fn interop_format(name: &str) -> Result<dv_interop::Format, String> {
    match name {
        "twb" => Ok(dv_interop::Format::Twb),
        "twbx" => Ok(dv_interop::Format::Twbx),
        "pbit" => Ok(dv_interop::Format::Pbit),
        "pbip" => Ok(dv_interop::Format::Pbip),
        "pbix" => Ok(dv_interop::Format::Pbix),
        other => Err(format!("unknown format '{other}'")),
    }
}

/// Files in the store at or under `path`.
fn files_under(path: &str) -> Vec<String> {
    let root = dv_interop::fsx::memfs::key(std::path::Path::new(path));
    let prefix = format!("{root}/");
    dv_interop::fsx::memfs::list()
        .into_iter()
        .filter(|f| *f == root || f.starts_with(&prefix))
        .collect()
}

/// Converts a Tableau or Power BI file already in the store (a file, or a project folder) to a
/// report. Data files packed inside a workbook are written under `out_dir`. Returns
/// `{"report", "warnings", "files"}`; `files` are the paths written under `out_dir`.
///
/// Calculated fields are not checked against the engine here: the page checks each one and
/// drops the ones the engine refuses, as the desktop app does.
#[wasm_bindgen]
pub fn interop_import(path: &str, out_dir: &str) -> String {
    respond(|| {
        let imported = dv_interop::import(
            std::path::Path::new(path),
            std::path::Path::new(out_dir),
            &dv_interop::ImportOptions::default(),
        )
        .map_err(|e| e.to_string())?;
        Ok(json!({
            "report": imported.report,
            "warnings": imported.warnings,
            "files": files_under(out_dir),
        }))
    })
}

/// Writes a report as `format` (`twb`, `twbx`, `pbit` or `pbip`) to `out` in the store.
/// The report's data files (its sources' paths) must be in the store for `twbx`, and for the
/// column types of Power BI models. Returns `{"files": [...], "warnings": [...]}` where `files`
/// are every path written (a project folder is many files).
#[wasm_bindgen]
pub fn interop_export(report_json: &str, format: &str, out: &str) -> String {
    respond(|| {
        let report: Report = parse("report", report_json)?;
        let format = interop_format(format)?;
        let exported = dv_interop::export(
            &report,
            std::path::Path::new(""),
            format,
            std::path::Path::new(out),
        )
        .map_err(|e| e.to_string())?;
        Ok(json!({ "files": files_under(out), "warnings": exported.warnings }))
    })
}

/// Writes a report as a `.pbix`, which holds its data. Two things come from the page because
/// they need the query engine: the values of every calculated column and of every column of a
/// custom-SQL dataset (stored as a snapshot), as text cells with blanks as empty strings; and
/// the columns of each dataset with their DuckDB types.
///
/// * `values_json`: `{dataset: {column: [cell, ...]}}`
/// * `columns_json`: `{dataset: [[column, duckdb_type], ...]}`
///
/// Returns `{"files", "warnings"}` like [`interop_export`].
#[wasm_bindgen]
pub fn interop_export_pbix(
    report_json: &str,
    out: &str,
    values_json: &str,
    columns_json: &str,
) -> String {
    respond(|| {
        let report: Report = parse("report", report_json)?;
        let values: BTreeMap<String, BTreeMap<String, Vec<String>>> =
            parse("column values", values_json)?;
        let columns: BTreeMap<String, Vec<(String, String)>> =
            parse("dataset columns", columns_json)?;
        let compute = |dataset: &str, column: &str| {
            values
                .get(dataset)
                .and_then(|by_column| by_column.get(column))
                .cloned()
                .ok_or_else(|| format!("no values were computed for {dataset}.{column}"))
        };
        let describe = |dataset: &str| {
            columns
                .get(dataset)
                .map(|list| {
                    list.iter()
                        .map(|(name, kind)| {
                            (name.clone(), dv_interop::model_type(kind).to_string())
                        })
                        .collect()
                })
                .ok_or_else(|| format!("no columns were described for {dataset}"))
        };
        let options = dv_interop::ExportOptions {
            column_values: Some(&compute),
            dataset_columns: Some(&describe),
        };
        let exported = dv_interop::export_with(
            &report,
            std::path::Path::new(""),
            dv_interop::Format::Pbix,
            std::path::Path::new(out),
            &options,
        )
        .map_err(|e| e.to_string())?;
        Ok(json!({ "files": files_under(out), "warnings": exported.warnings }))
    })
}

// ---------------------------------------------------------------------------
// SQLite database sources
// ---------------------------------------------------------------------------

/// Opens a SQLite file from its bytes (read only) and keeps it under `name`. Returns
/// `{"tables": [...]}`: its tables and views.
#[wasm_bindgen]
pub fn sqlite_open(name: &str, bytes: &[u8]) -> String {
    respond(|| {
        let tables = sqlite::open(name, bytes)?;
        Ok(json!({ "tables": tables }))
    })
}

/// One table or view of an opened SQLite database, as an Arrow IPC stream (throws on failure).
#[wasm_bindgen]
pub fn sqlite_table_ipc(name: &str, table: &str) -> Result<Vec<u8>, JsError> {
    sqlite::table_ipc(name, table).map_err(|e| JsError::new(&e))
}

/// Forgets a database opened with [`sqlite_open`], freeing its memory.
#[wasm_bindgen]
pub fn sqlite_close(name: &str) {
    sqlite::close(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../examples/sample.dvz");
    const JOINED: &str = include_str!("../../../examples/sample_joined.dvz");

    fn ok(text: String) -> Value {
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["ok"], true, "{value}");
        value
    }

    #[test]
    fn opens_a_sample_report() {
        let out = ok(open_report(SAMPLE));
        assert_eq!(out["report"]["name"], "Sample sales");
    }

    #[test]
    fn rejects_a_bad_report() {
        let out: Value =
            serde_json::from_str(&open_report("{\"version\": 99, \"name\": \"x\"}")).unwrap();
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("version"));
    }

    #[test]
    fn compiles_a_visual() {
        let report: Report = Report::from_json(SAMPLE).unwrap();
        let visual = serde_json::to_string(&report.visuals[0]).unwrap();
        let out = ok(compile_visual(&visual, "[]", "{}", "", "[]"));
        assert!(out["sql"].as_str().unwrap().starts_with("SELECT"));
        assert_eq!(out["visual"]["id"], report.visuals[0].id);
    }

    #[test]
    fn builds_the_joined_dataset_sql_from_relation_columns() {
        let report = Report::from_json(JOINED).unwrap();
        let dataset = report
            .datasets
            .iter()
            .find(|d| !d.joins.is_empty())
            .unwrap();
        let mut columns = BTreeMap::new();
        for source in &report.sources {
            columns.insert(source.name.clone(), vec!["id".to_string(), "x".to_string()]);
        }
        for d in &report.datasets {
            columns
                .entry(d.name.clone())
                .or_insert_with(|| vec!["id".to_string(), "x".to_string()]);
        }
        let out: Value = serde_json::from_str(&dataset_base_sql(
            &serde_json::to_string(dataset).unwrap(),
            &serde_json::to_string(&columns).unwrap(),
        ))
        .unwrap();
        // The sample joins on specific columns, so fake columns are reported as an error
        // that names the missing column rather than a panic.
        assert!(out["ok"] == true || out["error"].as_str().unwrap().contains("not found"));
    }

    #[test]
    fn layers_calculated_fields_and_rejects_clashes() {
        let dataset = json!({
            "name": "d", "source": "s",
            "calculated_fields": [{"name": "double", "expression": "units * 2"}]
        });
        let out = ok(dataset_layers(
            &dataset.to_string(),
            "SELECT * FROM \"s\"",
            "[\"units\"]",
        ));
        assert!(out["sql"].as_str().unwrap().contains("AS \"double\""));
        let clash: Value = serde_json::from_str(&dataset_layers(
            &dataset.to_string(),
            "SELECT * FROM \"s\"",
            "[\"Double\"]",
        ))
        .unwrap();
        assert_eq!(clash["ok"], false);
    }

    #[test]
    fn calc_precheck_rejects_statements() {
        let out: Value = serde_json::from_str(&calc_parse_query("f", "1; drop table x")).unwrap();
        assert_eq!(out["ok"], false);
    }

    #[test]
    fn parameter_values_must_be_an_object() {
        let report: Report = Report::from_json(SAMPLE).unwrap();
        let visual = serde_json::to_string(&report.visuals[0]).unwrap();
        let out: Value =
            serde_json::from_str(&compile_visual(&visual, "[]", "[1]", "", "[]")).unwrap();
        assert_eq!(out["ok"], false);
    }

    #[test]
    fn plans_an_export() {
        let out = ok(export_plan(SAMPLE, "{}", "{}", false));
        assert!(!out["items"].as_array().unwrap().is_empty());
    }
}
