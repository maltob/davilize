//! WebAssembly bindings. The browser build reads and checks `.dvz` reports and compiles
//! visuals to SQL with the same code as the CLI. The page runs that SQL in DuckDB-Wasm,
//! because the native DuckDB crate does not build for WebAssembly.

use dv_core::{Dataset, Report, bind_visual, resolve_parameters};
use serde_json::{Map, Value, json};
use wasm_bindgen::prelude::*;

/// Parses and validates a `.dvz` report and returns a JSON summary:
/// `{"ok": true, "report": {...}}` or `{"ok": false, "error": "..."}`.
#[wasm_bindgen]
pub fn inspect(text: &str) -> String {
    summarize(text).to_string()
}

/// The schema version this build reads.
#[wasm_bindgen]
pub fn report_version() -> u32 {
    dv_core::REPORT_VERSION
}

/// Builds the browser run plan for a report, as JSON:
/// `{"ok": true, "plan": {"files": [...], "views": [...], "visuals": [...]}}`.
///
/// * `files`: the data files the page must supply, `{source, path, format}`.
/// * `views`: SQL to run in order: one `CREATE VIEW` per source and dataset.
/// * `visuals`: per visual `{id, title, chart, encodings, sql}` or `{id, title, error}`.
///
/// `params_json` is an object of parameter values by name (`{}` for the defaults).
/// Joins and database sources are not supported in the browser; datasets that use them are
/// reported in `skipped`.
#[wasm_bindgen]
pub fn plan(text: &str, params_json: &str) -> String {
    build_plan(text, params_json).to_string()
}

fn summarize(text: &str) -> Value {
    match Report::from_json(text) {
        Ok(report) => json!({
            "ok": true,
            "report": {
                "name": report.name,
                "version": report.version,
                "sources": report.sources.iter().map(|s| &s.name).collect::<Vec<_>>(),
                "datasets": report.datasets.iter().map(|d| json!({
                    "name": d.name,
                    "source": d.source,
                    "sql": d.sql.is_some(),
                })).collect::<Vec<_>>(),
                "visuals": report.visuals.iter().map(|v| json!({
                    "id": v.id,
                    "title": v.title,
                    "dataset": v.dataset,
                    "chart": serde_json::to_value(v.chart).unwrap_or(Value::Null),
                })).collect::<Vec<_>>(),
                "parameters": report.parameters.len(),
                "dashboards": report.dashboards.iter().map(|d| &d.title).collect::<Vec<_>>(),
                "schedules": report.schedules.len(),
            }
        }),
        Err(err) => json!({ "ok": false, "error": err.to_string() }),
    }
}

fn failure(message: impl ToString) -> Value {
    json!({ "ok": false, "error": message.to_string() })
}

/// The SELECT that defines a dataset, or why the browser cannot build it.
fn dataset_body(dataset: &Dataset) -> Result<String, String> {
    if !dataset.joins.is_empty() {
        return Err("joins are not supported in the browser".into());
    }
    let mut body = match &dataset.sql {
        Some(sql) => sql.clone(),
        None => format!("SELECT * FROM {}", dv_sql::quote_ident(&dataset.source)),
    };
    for field in &dataset.calculated_fields {
        body = format!(
            "SELECT *, ({}) AS {} FROM ({body})",
            field.expression,
            dv_sql::quote_ident(&field.name)
        );
    }
    Ok(body)
}

fn build_plan(text: &str, params_json: &str) -> Value {
    let report = match Report::from_json(text) {
        Ok(report) => report,
        Err(err) => return failure(err),
    };
    let overrides: Map<String, Value> = match serde_json::from_str(params_json) {
        Ok(Value::Object(map)) => map,
        Ok(_) | Err(_) => return failure("parameter values must be a JSON object"),
    };
    let supplied: Vec<(String, String)> = overrides
        .into_iter()
        .map(|(name, value)| match value {
            Value::String(text) => (name, text),
            other => (name, other.to_string()),
        })
        .collect();
    let values = match resolve_parameters(&report.parameters, &supplied) {
        Ok(values) => values,
        Err(err) => return failure(err),
    };

    let mut files = Vec::new();
    let mut views = Vec::new();
    let mut skipped = Vec::new();
    for source in &report.sources {
        match serde_json::to_value(source) {
            Ok(Value::Object(map)) if map.get("type") == Some(&json!("file")) => {
                files.push(json!({
                    "source": source.name,
                    "path": map.get("path"),
                    "format": map.get("format"),
                }));
            }
            _ => skipped.push(json!({
                "name": source.name,
                "reason": "only file sources work in the browser",
            })),
        }
    }
    let mut usable = Vec::new();
    for dataset in &report.datasets {
        match dataset_body(dataset) {
            Ok(body) => {
                views.push(format!(
                    "CREATE OR REPLACE VIEW {} AS {body}",
                    dv_sql::quote_ident(&dataset.name)
                ));
                usable.push(dataset.name.clone());
            }
            Err(reason) => skipped.push(json!({ "name": dataset.name, "reason": reason })),
        }
    }

    let visuals: Vec<Value> = report
        .visuals
        .iter()
        .map(|visual| {
            let head = json!({
                "id": visual.id,
                "title": visual.title,
                "dataset": visual.dataset,
                "chart": serde_json::to_value(visual.chart).unwrap_or(Value::Null),
                "encodings": serde_json::to_value(&visual.encodings).unwrap_or(Value::Null),
            });
            let compiled = if usable.contains(&visual.dataset) {
                bind_visual(visual, &values)
                    .map_err(|e| e.to_string())
                    .and_then(|bound| dv_sql::compile_visual(&bound).map_err(|e| e.to_string()))
            } else {
                Err(format!(
                    "dataset '{}' is not available in the browser",
                    visual.dataset
                ))
            };
            let mut entry = head.as_object().cloned().unwrap_or_default();
            match compiled {
                Ok(sql) => entry.insert("sql".into(), json!(sql)),
                Err(error) => entry.insert("error".into(), json!(error)),
            };
            Value::Object(entry)
        })
        .collect();

    json!({
        "ok": true,
        "plan": { "name": report.name, "files": files, "views": views, "visuals": visuals, "skipped": skipped }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../../../examples/sample.dvz");

    #[test]
    fn summarizes_a_sample_report() {
        let out = summarize(SAMPLE);
        assert_eq!(out["ok"], true);
        assert_eq!(out["report"]["name"], "Sample sales");
        assert!(!out["report"]["visuals"].as_array().unwrap().is_empty());
    }

    #[test]
    fn reports_a_bad_file_as_an_error() {
        let out = summarize("{\"version\": 99, \"name\": \"x\"}");
        assert_eq!(out["ok"], false);
        assert!(out["error"].as_str().unwrap().contains("version"));
    }

    #[test]
    fn plans_the_sample_report() {
        let out = build_plan(SAMPLE, "{}");
        assert_eq!(out["ok"], true, "{out}");
        let plan = &out["plan"];
        assert_eq!(plan["files"][0]["path"], "sample.csv");
        assert!(
            plan["views"][0]
                .as_str()
                .unwrap()
                .starts_with("CREATE OR REPLACE VIEW")
        );
        let visuals = plan["visuals"].as_array().unwrap();
        assert!(visuals.iter().all(|v| v["sql"].is_string()), "{visuals:?}");
    }

    #[test]
    fn joins_are_skipped_with_a_reason() {
        let text = include_str!("../../../examples/sample_joined.dvz");
        let out = build_plan(text, "{}");
        assert_eq!(out["ok"], true, "{out}");
        assert!(!out["plan"]["skipped"].as_array().unwrap().is_empty());
        assert!(out["plan"]["visuals"][0]["error"].is_string());
    }

    #[test]
    fn parameter_values_must_be_an_object() {
        assert_eq!(build_plan(SAMPLE, "[1]")["ok"], false);
    }
}
