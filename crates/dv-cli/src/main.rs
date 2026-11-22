//! `davilize` command line: inspect data files, run SQL, and refresh report outputs.
//!
//! Exit codes:
//! * 0 success
//! * 2 invalid command line (reported by clap)
//! * 3 input problem: report or data file missing or unreadable, invalid report,
//!   invalid `--param` (malformed, unknown name, value of the wrong type or outside
//!   its constraints, or a required parameter with no value), invalid publish target
//!   or file name, or serve folder missing
//! * 4 query problem: SQL error or a visual spec that cannot be compiled
//! * 5 output problem: cannot create or write the output directory or files
//! * 6 publish or network problem: missing or inconsistent credentials, feature not
//!   compiled in, upload refused or interrupted, or the server cannot listen

mod pipeline;
mod publish_cmd;
mod serve_cmd;

use std::fmt::Display;
use std::io::{self, Write};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand, ValueEnum};
use duckdb::arrow::array::Array;
use duckdb::arrow::datatypes::DataType;
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::display::array_value_to_string;
use duckdb::arrow::util::pretty::pretty_format_batches;
use dv_core::{CoreError, Report};
use dv_engine::sql::quote_ident;
use dv_engine::{Engine, EngineError, OutputFormat};
use serde_json::{Map, Value, json};

use publish_cmd::PublishFormat;
use serve_cmd::RefreshPlan;

const EXIT_INPUT: u8 = 3;
const EXIT_QUERY: u8 = 4;
const EXIT_OUTPUT: u8 = 5;
const EXIT_PUBLISH: u8 = 6;

#[derive(Parser)]
#[command(
    name = "davilize",
    version,
    about = "Headless data exploration and report refresh"
)]
struct Cli {
    /// Write log events as JSON lines on stderr (for schedulers and log collectors).
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show the schema and column profile of a data file (CSV, Parquet, JSON, Arrow IPC).
    Inspect {
        /// Data file to inspect.
        file: PathBuf,
    },
    /// Run a SQL query over a data file or a .dvz report's sources and datasets.
    ///
    /// A data file (CSV, Parquet, JSON, Arrow IPC: .arrow/.feather) is exposed as a
    /// view named after its file stem. A database source of a report is read-only;
    /// its connection comes from the environment variable DAVILIZE_CONN_<NAME>
    /// (SQLite: a file path; Postgres: a connection string), and its tables are
    /// exposed as "<source>"."<table>".
    ///
    /// With a .dvz report, --param binds its parameters, and each declared parameter
    /// is then available in SQL as a macro: "param_<name>"() returns its typed value.
    RunSql {
        /// A data file, or a .dvz report.
        input: PathBuf,
        /// The SQL to run.
        sql: String,
        /// Output format written to stdout.
        #[arg(long, value_enum, default_value_t = ResultFormat::Table)]
        format: ResultFormat,
        /// Parameter value, NAME=VALUE (repeatable). Only for a .dvz report.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
    /// Run every visual of a report and write its outputs to the output directory.
    ///
    /// Per visual: csv, parquet (data), svg, png, pdf (every chart type).
    /// Per report: html (all visuals on one page) and xlsx (one sheet per visual),
    /// written as <report-file-stem>.html and <report-file-stem>.xlsx.
    Refresh {
        /// The .dvz report to refresh.
        report: PathBuf,
        /// Output directory (created if missing).
        #[arg(long)]
        out: PathBuf,
        /// Comma-separated output formats.
        #[arg(long, value_enum, value_delimiter = ',', default_value = "csv")]
        format: Vec<DataOutput>,
        /// Inline vega, vega-lite and vega-embed into the HTML report so it works
        /// without network access. Requires --vendor-dir.
        #[arg(long, requires = "vendor_dir")]
        offline: bool,
        /// Directory with vega.min.js, vega-lite.min.js and vega-embed.min.js
        /// (the UMD builds from the npm packages vega@6.4.0, vega-lite@6.5.0,
        /// vega-embed@7.3.0, in their build/ folders).
        #[arg(long, value_name = "DIR")]
        vendor_dir: Option<PathBuf>,
        /// Parameter value, NAME=VALUE (repeatable). Defaults apply to the rest.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
    /// Refresh a report into a temporary folder, add manifest.json, and upload it.
    ///
    /// Target: a local folder (plain path or file://), s3://bucket/prefix (S3 and
    /// S3-compatible stores, endpoint from AWS_ENDPOINT_URL), or
    /// sftp://user@host[:port]/path (built in, pure Rust, no system ssh; Windows, Linux, macOS).
    /// Credentials come from the environment only; see docs/publishing.md.
    /// index.html is always included and is uploaded last.
    Publish {
        /// The .dvz report to refresh and publish.
        report: PathBuf,
        /// Destination URL or local path.
        #[arg(long, value_name = "URL")]
        target: String,
        /// Comma-separated formats to include (html is always included).
        #[arg(long, value_enum, value_delimiter = ',', default_value = "html")]
        format: Vec<PublishFormat>,
        /// Inline the vega libraries into the HTML so it works without network access.
        /// Requires --vendor-dir.
        #[arg(long, requires = "vendor_dir")]
        offline: bool,
        /// Directory with the vendored vega, vega-lite and vega-embed UMD bundles.
        #[arg(long, value_name = "DIR")]
        vendor_dir: Option<PathBuf>,
        /// List the files and their sizes and digests without connecting to the target.
        #[arg(long)]
        dry_run: bool,
        /// SFTP: record an unknown server host key in known_hosts on first use.
        #[arg(long)]
        trust_new_host_key: bool,
        /// Parameter value, NAME=VALUE (repeatable). Defaults apply to the rest.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
    /// Inspect the parameters a report declares.
    Param {
        #[command(subcommand)]
        command: ParamCommand,
    },
    /// Serve a published folder over HTTP (static files, security headers).
    ///
    /// Binds to 127.0.0.1 unless --bind names another address. With --refresh,
    /// <dir> holds generations of output: the report is re-run every --every seconds
    /// (default 300) and on POST /refresh (needs DAVILIZE_REFRESH_TOKEN; disabled when
    /// unset). A new generation replaces the served one in one step.
    Serve {
        /// The folder to serve (with --refresh: the folder that holds the generations).
        dir: PathBuf,
        /// Port to listen on (0 picks a free port and prints it).
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// Address to bind. Non-loopback addresses are logged with a warning.
        #[arg(long, value_name = "ADDR")]
        bind: Option<IpAddr>,
        /// Report to re-run in the background.
        #[arg(long, value_name = "REPORT")]
        refresh: Option<PathBuf>,
        /// Seconds between background refreshes (requires --refresh).
        #[arg(long, value_name = "SECONDS", requires = "refresh")]
        every: Option<u64>,
        /// Comma-separated formats written by each refresh (html is the served page).
        #[arg(long, value_enum, value_delimiter = ',', default_value = "html")]
        format: Vec<DataOutput>,
        /// Inline the vega libraries into the HTML. Requires --vendor-dir.
        #[arg(long, requires = "vendor_dir")]
        offline: bool,
        /// Directory with the vendored vega, vega-lite and vega-embed UMD bundles.
        #[arg(long, value_name = "DIR")]
        vendor_dir: Option<PathBuf>,
        /// Parameter value, NAME=VALUE (repeatable). Needs --refresh. Checked at
        /// startup, and on every background refresh.
        #[arg(long = "param", value_name = "NAME=VALUE", requires = "refresh")]
        params: Vec<String>,
    },
}

#[derive(Subcommand)]
enum ParamCommand {
    /// List a report's parameters: name, type, default, constraints and description.
    ///
    /// With the global --json flag, prints a JSON array on stdout instead of a table.
    List {
        /// The .dvz report.
        report: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ResultFormat {
    Table,
    Csv,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DataOutput {
    Csv,
    Parquet,
    Html,
    Svg,
    Png,
    Pdf,
    Xlsx,
}

impl DataOutput {
    /// The engine writes these straight from SQL, one file per visual.
    fn engine_format(self) -> Option<OutputFormat> {
        match self {
            DataOutput::Csv => Some(OutputFormat::Csv),
            DataOutput::Parquet => Some(OutputFormat::Parquet),
            _ => None,
        }
    }
}

/// A failure with the process exit code it maps to.
struct CliError {
    code: u8,
    message: String,
}

type CliResult<T> = Result<T, CliError>;

impl From<dv_render::RenderError> for CliError {
    fn from(err: dv_render::RenderError) -> Self {
        let code = match err {
            dv_render::RenderError::Output(_) => EXIT_OUTPUT,
            _ => EXIT_QUERY,
        };
        CliError {
            code,
            message: err.to_string(),
        }
    }
}

/// Parameter problems are input problems (exit 3). The core reports them as `Invalid`,
/// so they get their own wording here.
fn param_error(err: CoreError) -> CliError {
    let message = match err {
        CoreError::Invalid(reason) => format!("invalid parameter: {reason}"),
        other => other.to_string(),
    };
    CliError {
        code: EXIT_INPUT,
        message,
    }
}

impl From<CoreError> for CliError {
    fn from(err: CoreError) -> Self {
        CliError {
            code: EXIT_INPUT,
            message: err.to_string(),
        }
    }
}

impl From<EngineError> for CliError {
    fn from(err: EngineError) -> Self {
        let code = match err {
            EngineError::Io { .. }
            | EngineError::MissingConnection { .. }
            | EngineError::Attach { .. }
            | EngineError::Extension { .. } => EXIT_INPUT,
            EngineError::Duckdb(_) | EngineError::InvalidSpec(_) | EngineError::Unsupported(_) => {
                EXIT_QUERY
            }
        };
        CliError {
            code,
            message: err.to_string(),
        }
    }
}

fn output_error(err: impl Display) -> CliError {
    CliError {
        code: EXIT_OUTPUT,
        message: err.to_string(),
    }
}

/// Structured logging: human lines by default, JSON lines with `--json`.
#[derive(Clone, Copy)]
struct Log {
    json: bool,
}

impl Log {
    fn info(&self, message: &str, fields: Value) {
        self.emit("info", message, fields);
    }

    fn warn(&self, message: &str, fields: Value) {
        self.emit("warn", message, fields);
    }

    fn error(&self, message: &str) {
        self.emit("error", message, Value::Null);
    }

    fn emit(&self, level: &str, message: &str, fields: Value) {
        if self.json {
            let ts_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let mut event = Map::new();
            event.insert("ts_ms".into(), json!(ts_ms));
            event.insert("level".into(), json!(level));
            event.insert("msg".into(), json!(message));
            if let Value::Object(extra) = fields {
                event.extend(extra);
            }
            eprintln!("{}", Value::Object(event));
        } else {
            eprintln!("[{level}] {message}");
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let log = Log { json: cli.json };
    let result = match cli.command {
        Command::Inspect { file } => inspect(&file, &log),
        Command::RunSql {
            input,
            sql,
            format,
            params,
        } => parse_params(&params).and_then(|params| run_sql(&input, &sql, format, &params, &log)),
        Command::Refresh {
            report,
            out,
            format,
            offline,
            vendor_dir,
            params,
        } => parse_params(&params).and_then(|params| {
            refresh(
                &report,
                &out,
                &format,
                offline.then_some(vendor_dir.as_deref()).flatten(),
                params,
                &log,
            )
        }),
        Command::Publish {
            report,
            target,
            format,
            offline,
            vendor_dir,
            dry_run,
            trust_new_host_key,
            params,
        } => parse_params(&params).and_then(|params| {
            publish_cmd::publish_report(
                publish_cmd::PublishRequest {
                    report: &report,
                    target: &target,
                    formats: &format,
                    vendor_dir: offline.then_some(vendor_dir.as_deref()).flatten(),
                    dry_run,
                    trust_new_host_key,
                    params,
                },
                &log,
            )
        }),
        Command::Param { command } => match command {
            ParamCommand::List { report } => param_list(&report, &log),
        },
        Command::Serve {
            dir,
            port,
            bind,
            refresh,
            every,
            format,
            offline,
            vendor_dir,
            params,
        } => parse_params(&params).and_then(|params| {
            let plan = refresh.map(|report| RefreshPlan {
                report,
                every: Duration::from_secs(every.unwrap_or(300).max(1)),
                options: pipeline::Options {
                    formats: format,
                    vendor_dir: offline.then_some(vendor_dir).flatten(),
                    html_name: Some(dv_publish::INDEX_NAME.to_string()),
                    params,
                },
            });
            serve_cmd::serve(&dir, port, bind, plan, &log)
        }),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log.error(&err.message);
            ExitCode::from(err.code)
        }
    }
}

fn file_stem_name(path: &Path) -> CliResult<String> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(str::to_string)
        .ok_or_else(|| CliError {
            code: EXIT_INPUT,
            message: format!("cannot derive a name from '{}'", path.display()),
        })
}

fn base_dir(report: &Path) -> &Path {
    report
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn is_report(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("dvz"))
}

/// Registers either a whole `.dvz` report or a single data file as views.
fn load_inputs(engine: &Engine, input: &Path) -> CliResult<()> {
    if is_report(input) {
        let report = Report::load(input)?;
        engine.register_report(&report, base_dir(input))?;
    } else {
        engine.register_file(&file_stem_name(input)?, input)?;
    }
    Ok(())
}

fn inspect(file: &Path, log: &Log) -> CliResult<()> {
    let engine = Engine::new()?;
    let name = file_stem_name(file)?;
    engine.register_file(&name, file)?;

    let schema = engine.query(&format!("DESCRIBE {}", quote_ident(&name)))?;
    println!("Schema of {}", file.display());
    print_table(&schema)?;

    let profile = engine.profile(&name)?;
    println!();
    println!("Profile of {}", file.display());
    print_table(&profile)?;

    let columns: usize = schema.iter().map(|b| b.num_rows()).sum();
    log.info(
        &format!("inspected {} ({columns} columns)", file.display()),
        json!({"file": file.display().to_string(), "columns": columns}),
    );
    Ok(())
}

/// Splits repeated `NAME=VALUE` arguments. Names must be non-empty; values may contain `=`.
fn parse_params(raw: &[String]) -> CliResult<Vec<(String, String)>> {
    raw.iter()
        .map(|item| match item.split_once('=') {
            Some((name, value)) if !name.is_empty() => Ok((name.to_string(), value.to_string())),
            _ => Err(CliError {
                code: EXIT_INPUT,
                message: format!("invalid --param '{item}': expected NAME=VALUE"),
            }),
        })
        .collect()
}

fn run_sql(
    input: &Path,
    sql: &str,
    format: ResultFormat,
    params: &[(String, String)],
    log: &Log,
) -> CliResult<()> {
    let engine = Engine::new()?;
    if is_report(input) {
        let report = Report::load(input)?;
        engine.register_report(&report, base_dir(input))?;
        let values =
            dv_core::resolve_parameters(&report.parameters, params).map_err(param_error)?;
        engine.define_parameter_macros(&report.parameters, &values)?;
    } else {
        if !params.is_empty() {
            return Err(CliError {
                code: EXIT_INPUT,
                message: "--param applies only to a .dvz report".to_string(),
            });
        }
        load_inputs(&engine, input)?;
    }
    let batches = engine.query(sql)?;
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();

    match format {
        ResultFormat::Table => print_table(&batches)?,
        ResultFormat::Csv => {
            let stdout = io::stdout();
            let mut out = stdout.lock();
            write_csv(&batches, &mut out).map_err(output_error)?;
            out.flush().map_err(output_error)?;
        }
        ResultFormat::Json => println!("{}", to_json(&batches)?),
    }
    log.info(
        &format!("query returned {rows} rows"),
        json!({"rows": rows}),
    );
    Ok(())
}

fn refresh(
    report_path: &Path,
    out_dir: &Path,
    formats: &[DataOutput],
    vendor_dir: Option<&Path>,
    params: Vec<(String, String)>,
    log: &Log,
) -> CliResult<()> {
    let options = pipeline::Options {
        formats: formats.to_vec(),
        vendor_dir: vendor_dir.map(Path::to_path_buf),
        html_name: None,
        params,
    };
    pipeline::run(report_path, out_dir, &options, log)?;
    Ok(())
}

/// `davilize param list`: one row per declared parameter.
fn param_list(report_path: &Path, log: &Log) -> CliResult<()> {
    let report = Report::load(report_path)?;
    let rows: Vec<Vec<String>> = report
        .parameters
        .iter()
        .map(|p| {
            vec![
                p.name.clone(),
                p.param_type().as_str().to_string(),
                match &p.default {
                    Some(default) => default.to_string(),
                    None => "(required)".to_string(),
                },
                constraints_text(p),
                p.description.clone().unwrap_or_default(),
            ]
        })
        .collect();

    if log.json {
        let items: Vec<Value> = report
            .parameters
            .iter()
            .map(|p| {
                json!({
                    "name": p.name,
                    "type": p.param_type().as_str(),
                    "default": p.default,
                    "required": p.default.is_none(),
                    "allowed": p.allowed,
                    "min": p.min,
                    "max": p.max,
                    "description": p.description,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&Value::Array(items)).map_err(output_error)?
        );
    } else if rows.is_empty() {
        println!("(no parameters)");
    } else {
        print_columns(
            &["NAME", "TYPE", "DEFAULT", "CONSTRAINTS", "DESCRIPTION"],
            &rows,
        );
    }
    log.info(
        &format!("{} parameters in '{}'", rows.len(), report.name),
        json!({"report": report.name, "parameters": rows.len()}),
    );
    Ok(())
}

/// Human-readable constraints: allowed values, then the bounds.
fn constraints_text(p: &dv_core::Parameter) -> String {
    let mut parts = Vec::new();
    if let Some(allowed) = &p.allowed {
        let items: Vec<String> = allowed.iter().map(|v| v.to_string()).collect();
        parts.push(format!("one of {}", items.join(", ")));
    }
    if let Some(min) = &p.min {
        parts.push(format!("min {min}"));
    }
    if let Some(max) = &p.max {
        parts.push(format!("max {max}"));
    }
    parts.join("; ")
}

/// Prints rows as left-aligned columns, each padded to its widest cell.
fn print_columns(header: &[&str], rows: &[Vec<String>]) {
    let mut widths: Vec<usize> = header.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| {
        let padded: Vec<String> = cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<width$}", width = widths[i]))
            .collect();
        println!("{}", padded.join("  ").trim_end());
    };
    line(&header.iter().map(|h| h.to_string()).collect::<Vec<_>>());
    for row in rows {
        line(row);
    }
}

fn write_output(path: &Path, bytes: &[u8]) -> CliResult<()> {
    std::fs::write(path, bytes)
        .map_err(|e| output_error(format!("cannot write '{}': {e}", path.display())))
}

/// Prints batches as a bordered text table on stdout.
fn print_table(batches: &[RecordBatch]) -> CliResult<()> {
    if batches.is_empty() {
        println!("(no result)");
        return Ok(());
    }
    let rendered = pretty_format_batches(batches).map_err(output_error)?;
    println!("{rendered}");
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("({rows} rows)");
    Ok(())
}

/// Writes batches as RFC 4180 CSV with a header row.
fn write_csv(batches: &[RecordBatch], out: &mut impl Write) -> io::Result<()> {
    let Some(first) = batches.first() else {
        return Ok(());
    };
    let header: Vec<String> = first
        .schema()
        .fields()
        .iter()
        .map(|f| csv_escape(f.name()))
        .collect();
    writeln!(out, "{}", header.join(","))?;
    for batch in batches {
        for row in 0..batch.num_rows() {
            let cells: Vec<String> = batch
                .columns()
                .iter()
                .map(|col| csv_escape(&cell_text(col.as_ref(), row)))
                .collect();
            writeln!(out, "{}", cells.join(","))?;
        }
    }
    Ok(())
}

fn csv_escape(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

fn cell_text(array: &dyn Array, row: usize) -> String {
    if array.is_null(row) {
        return String::new();
    }
    array_value_to_string(array, row).unwrap_or_default()
}

/// Converts batches to a JSON array of row objects, keeping numbers and booleans typed.
fn to_json(batches: &[RecordBatch]) -> CliResult<String> {
    let mut rows = Vec::new();
    for batch in batches {
        let schema = batch.schema();
        for row in 0..batch.num_rows() {
            let mut object = Map::new();
            for (index, field) in schema.fields().iter().enumerate() {
                let array = batch.column(index).as_ref();
                let value = if array.is_null(row) {
                    Value::Null
                } else {
                    let text = cell_text(array, row);
                    match field.data_type() {
                        DataType::Boolean => Value::Bool(text == "true"),
                        dt if dt.is_numeric() => {
                            serde_json::from_str(&text).unwrap_or(Value::String(text))
                        }
                        _ => Value::String(text),
                    }
                };
                object.insert(field.name().clone(), value);
            }
            rows.push(Value::Object(object));
        }
    }
    serde_json::to_string_pretty(&Value::Array(rows)).map_err(output_error)
}
