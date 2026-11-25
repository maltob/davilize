//! Import and export of Tableau and Power BI files.
//!
//! Conversions are best effort: anything the Davilize model cannot express is
//! reported as a warning instead of failing the whole conversion.

pub mod expr;
mod powerbi;
mod tableau;
mod util;

use std::path::{Path, PathBuf};

use dv_core::Report;

/// What can go wrong while importing or exporting.
#[derive(Debug, thiserror::Error)]
pub enum InteropError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Archive(String),
    #[error("{0}")]
    Format(String),
    #[error("{0}")]
    Unsupported(String),
    #[error("the converted report is not valid: {0}")]
    Invalid(String),
}

pub(crate) fn io_err(path: &Path) -> impl FnOnce(std::io::Error) -> InteropError + '_ {
    move |source| InteropError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// A file format this crate can read or write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Tableau workbook (XML).
    Twb,
    /// Packaged Tableau workbook (zip with data files).
    Twbx,
    /// Power BI report file; import reads the report layout only.
    Pbix,
    /// Power BI template (zip with the model schema and report layout).
    Pbit,
    /// Power BI Project folder (TMDL model + PBIR report).
    Pbip,
}

impl Format {
    /// Detects the format of an existing file or folder.
    pub fn detect(path: &Path) -> Option<Format> {
        if path.is_dir() {
            return Some(Format::Pbip);
        }
        Self::from_extension(path)
    }

    /// Detects a format from the extension alone (for files that do not exist yet).
    pub fn from_extension(path: &Path) -> Option<Format> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "twb" => Some(Format::Twb),
            "twbx" => Some(Format::Twbx),
            "pbix" => Some(Format::Pbix),
            "pbit" => Some(Format::Pbit),
            "pbip" => Some(Format::Pbip),
            _ => None,
        }
    }
}

/// Checks a translated expression against the engine's calculated-field rules.
/// Arguments: field name and expression. `Err` carries the reason.
pub type ExpressionCheck<'a> = &'a dyn Fn(&str, &str) -> Result<(), String>;

#[derive(Default, Clone, Copy)]
pub struct ImportOptions<'a> {
    /// When set, every translated expression is passed through it and dropped
    /// (with a warning) if it is refused.
    pub check_expression: Option<ExpressionCheck<'a>>,
}

/// The outcome of an import.
#[derive(Debug)]
pub struct Imported {
    pub report: Report,
    /// Things that could not be converted, one sentence each.
    pub warnings: Vec<String>,
    /// Data files written next to the report (unpacked from the workbook).
    pub files: Vec<PathBuf>,
}

/// The outcome of an export.
#[derive(Debug)]
pub struct Exported {
    /// Every file or folder written.
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

/// Imports a Tableau or Power BI file (or Power BI Project folder).
///
/// Data files found inside packaged workbooks are unpacked into `out_dir`, and the
/// report's source paths are relative to it, so save the report there.
pub fn import(
    path: &Path,
    out_dir: &Path,
    options: &ImportOptions<'_>,
) -> Result<Imported, InteropError> {
    let format = Format::detect(path).ok_or_else(|| {
        InteropError::Unsupported(format!(
            "{}: not a Tableau (.twb, .twbx) or Power BI (.pbix, .pbit, project folder) file",
            path.display()
        ))
    })?;
    std::fs::create_dir_all(out_dir).map_err(io_err(out_dir))?;
    let imported = match format {
        Format::Twb | Format::Twbx => tableau::import(path, out_dir, options)?,
        Format::Pbix | Format::Pbit | Format::Pbip => {
            powerbi::import(path, format, out_dir, options)?
        }
    };
    imported
        .report
        .validate()
        .map_err(|e| InteropError::Invalid(e.to_string()))?;
    Ok(imported)
}

/// Computes the values of a calculated column: `(dataset, column)` to one text cell per
/// row, or why it cannot. A `.pbix` stores calculated columns with their data, so the
/// exporter needs an engine to evaluate them; without one such a report cannot be
/// exported to `.pbix`.
pub type ColumnValues<'a> = &'a dyn Fn(&str, &str) -> Result<Vec<String>, String>;

/// The model type for a DuckDB column type, as the engine names it.
pub fn model_type(duckdb_type: &str) -> &'static str {
    let t = duckdb_type.to_ascii_uppercase();
    match t.as_str() {
        "BOOLEAN" => "boolean",
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT"
        | "UINTEGER" | "UBIGINT" => "int64",
        "FLOAT" | "DOUBLE" | "REAL" => "double",
        "DATE" => "dateTime",
        _ if t.starts_with("DECIMAL") => "double",
        _ if t.starts_with("TIMESTAMP") => "dateTime",
        _ => "string",
    }
}

/// The columns of a dataset as `(name, type)` pairs, with types named as the model
/// names them (`string`, `int64`, `double`, `dateTime`, `boolean`). A dataset that
/// cannot be flattened to a CSV file (custom SQL, say) is exported to `.pbix` as a
/// snapshot of its rows, which needs the engine that can run it.
pub type DatasetColumns<'a> = &'a dyn Fn(&str) -> Result<Vec<(String, String)>, String>;

/// Options for [`export_with`].
#[derive(Default)]
pub struct ExportOptions<'a> {
    pub column_values: Option<ColumnValues<'a>>,
    pub dataset_columns: Option<DatasetColumns<'a>>,
}

/// Exports a report. `report_dir` is the folder the report was loaded from (its
/// relative data paths are resolved against it). For Power BI Project output `out`
/// is a folder; otherwise it is the file to write.
pub fn export(
    report: &Report,
    report_dir: &Path,
    format: Format,
    out: &Path,
) -> Result<Exported, InteropError> {
    export_with(report, report_dir, format, out, &ExportOptions::default())
}

/// [`export`] with options.
pub fn export_with(
    report: &Report,
    report_dir: &Path,
    format: Format,
    out: &Path,
    options: &ExportOptions<'_>,
) -> Result<Exported, InteropError> {
    report
        .validate()
        .map_err(|e| InteropError::Invalid(e.to_string()))?;
    match format {
        Format::Twb => tableau::export(report, report_dir, false, out),
        Format::Twbx => tableau::export(report, report_dir, true, out),
        Format::Pbit => powerbi::export_pbit(report, report_dir, out),
        Format::Pbip => powerbi::export_pbip(report, report_dir, out),
        Format::Pbix => powerbi::export_pbix(report, report_dir, out, options),
    }
}
