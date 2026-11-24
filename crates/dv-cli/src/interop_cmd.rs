//! `davilize import` and `davilize export`: Tableau and Power BI files.

use std::path::{Path, PathBuf};

use clap::ValueEnum;
use dv_core::Report;
use dv_engine::Engine;
use dv_interop::{Format, ImportOptions, InteropError};
use serde_json::json;

use crate::{CliError, CliResult, EXIT_INPUT, EXIT_OUTPUT, Log};

#[derive(Clone, Copy, ValueEnum)]
pub enum InteropFormat {
    /// Tableau workbook (XML).
    Twb,
    /// Packaged Tableau workbook, with the data files inside.
    Twbx,
    /// Power BI template; open it in Power BI Desktop and save as .pbix.
    Pbit,
    /// Power BI Project folder (TMDL model + PBIR report).
    Pbip,
}

impl InteropFormat {
    fn format(self) -> Format {
        match self {
            InteropFormat::Twb => Format::Twb,
            InteropFormat::Twbx => Format::Twbx,
            InteropFormat::Pbit => Format::Pbit,
            InteropFormat::Pbip => Format::Pbip,
        }
    }
}

fn interop_error(err: InteropError, code: u8) -> CliError {
    CliError {
        code,
        message: err.to_string(),
    }
}

/// Imports `input` and writes the report to `out`. Data files found inside the
/// input are unpacked next to `out`.
pub fn import_file(input: &Path, out: &Path, log: &Log) -> CliResult<()> {
    let out_dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let engine = Engine::new()?;
    let check = |field: &str, expression: &str| {
        engine
            .check_calculated_expression(field, expression)
            .map_err(|e| e.to_string())
    };
    let options = ImportOptions {
        check_expression: Some(&check),
    };
    let imported =
        dv_interop::import(input, &out_dir, &options).map_err(|e| interop_error(e, EXIT_INPUT))?;
    imported.report.save(out).map_err(|e| CliError {
        code: EXIT_OUTPUT,
        message: e.to_string(),
    })?;
    for warning in &imported.warnings {
        log.warn(warning, json!({}));
    }
    log.info(
        &format!(
            "imported {} into {}: {} source(s), {} dataset(s), {} visual(s), {} dashboard(s), {} warning(s)",
            input.display(),
            out.display(),
            imported.report.sources.len(),
            imported.report.datasets.len(),
            imported.report.visuals.len(),
            imported.report.dashboards.len(),
            imported.warnings.len()
        ),
        json!({
            "report": out.display().to_string(),
            "warnings": imported.warnings.len(),
            "data_files": imported.files.len(),
        }),
    );
    Ok(())
}

/// Exports the report at `input` as `format` to `out`.
pub fn export_file(input: &Path, format: InteropFormat, out: &Path, log: &Log) -> CliResult<()> {
    let report = Report::load(input)?;
    let report_dir = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
    let exported = dv_interop::export(&report, &report_dir, format.format(), out)
        .map_err(|e| interop_error(e, EXIT_OUTPUT))?;
    for warning in &exported.warnings {
        log.warn(warning, json!({}));
    }
    log.info(
        &format!(
            "exported {} to {} ({} warning(s))",
            input.display(),
            out.display(),
            exported.warnings.len()
        ),
        json!({ "output": out.display().to_string(), "warnings": exported.warnings.len() }),
    );
    Ok(())
}
