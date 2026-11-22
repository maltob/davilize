//! Query engine for Davilize, backed by bundled DuckDB.
//!
//! Sources and datasets are registered as DuckDB views, queries return Arrow
//! `RecordBatch`es, and visual specs are compiled to SQL by [`sql::compile_visual`].

mod arrow_ipc;
mod calc;
#[cfg(test)]
mod calc_tests;
mod database;
mod error;
pub mod sql;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use duckdb::Connection;
use duckdb::arrow::array::Array;
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::arrow::util::display::array_value_to_string;
use duckdb::vtab::arrow::ArrowVTab;
use dv_core::{
    Dataset, DbKind, FileFormat, ParamType, ParamValues, Parameter, Report, Source, SourceKind,
    Visual,
};

pub use database::{connection_env_var, resolve_connection};
pub use error::EngineError;
pub use sql::{compile_visual, compile_visual_with};

/// Column metadata as reported by DuckDB's `DESCRIBE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnInfo {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    /// True for columns added by a dataset's calculated fields.
    pub calculated: bool,
}

/// Output formats supported by [`Engine::export`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Csv,
    Parquet,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Csv => "csv",
            OutputFormat::Parquet => "parquet",
        }
    }
}

/// An in-process DuckDB session with registered sources and datasets.
pub struct Engine {
    conn: Connection,
    /// DuckDB extensions already loaded in this session.
    extensions: RefCell<HashSet<&'static str>>,
    /// Names of database sources, which expose several tables and cannot be selected directly.
    database_sources: RefCell<HashSet<String>>,
    /// Per dataset: the lowercase names of its calculated fields.
    calculated: RefCell<HashMap<String, HashSet<String>>>,
}

impl Engine {
    /// Opens a new in-memory engine.
    pub fn new() -> Result<Self, EngineError> {
        let conn = Connection::open_in_memory()?;
        // Makes `arrow(?)` available for querying in-memory Arrow batches.
        conn.register_table_function::<ArrowVTab>("arrow")?;
        Ok(Self {
            conn,
            extensions: RefCell::new(HashSet::new()),
            database_sources: RefCell::new(HashSet::new()),
            calculated: RefCell::new(HashMap::new()),
        })
    }

    /// Registers every source and dataset of `report`. Relative file paths
    /// are resolved against `base_dir` (normally the report's directory).
    pub fn register_report(&self, report: &Report, base_dir: &Path) -> Result<(), EngineError> {
        for source in &report.sources {
            self.register_source(source, base_dir)?;
        }
        for dataset in &report.datasets {
            self.register_dataset(dataset)?;
        }
        Ok(())
    }

    /// Registers a source. A file source becomes a view named after the source.
    /// A database source resolves its connection reference from the environment
    /// (see [`resolve_connection`]) and exposes its tables as views in a schema
    /// named after the source, e.g. `"warehouse"."orders"`.
    pub fn register_source(&self, source: &Source, base_dir: &Path) -> Result<(), EngineError> {
        match &source.kind {
            SourceKind::File { path, format } => {
                let full = if path.is_absolute() {
                    path.clone()
                } else {
                    base_dir.join(path)
                };
                if !full.exists() {
                    return Err(EngineError::Io {
                        path: full.clone(),
                        source: std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"),
                    });
                }
                if *format == FileFormat::Arrow {
                    return self.register_arrow_file(&source.name, &full);
                }
                let reader = reader_function(*format)?;
                let sql = format!(
                    "CREATE OR REPLACE VIEW {} AS SELECT * FROM {reader}({})",
                    sql::quote_ident(&source.name),
                    sql::quote_literal(&full.to_string_lossy()),
                );
                self.conn.execute_batch(&sql)?;
                Ok(())
            }
            SourceKind::Database { kind, connection } => {
                let target = resolve_connection(connection)?;
                self.register_database(&source.name, *kind, &target)
            }
        }
    }

    /// Attaches a database read-only and exposes its tables as views in a schema
    /// named `source`. `target` is the SQLite file path or the Postgres connection
    /// string. [`Engine::register_source`] takes it from the environment; callers
    /// that already hold the target can call this directly.
    pub fn register_database(
        &self,
        source: &str,
        kind: DbKind,
        target: &str,
    ) -> Result<(), EngineError> {
        let (extension, attach_type) = match kind {
            DbKind::Sqlite => ("sqlite", "sqlite"),
            DbKind::Postgres => ("postgres", "postgres"),
        };
        self.load_extension(extension)?;

        let catalog = format!("dv_db_{source}");
        let alias = sql::quote_ident(&catalog);
        let schema = sql::quote_ident(source);
        self.conn
            .execute_batch(&format!("DETACH DATABASE IF EXISTS {alias}"))?;
        let attach = format!(
            "ATTACH {} AS {alias} (TYPE {attach_type}, READ_ONLY)",
            sql::quote_literal(target)
        );
        self.conn
            .execute_batch(&attach)
            .map_err(|e| EngineError::Attach {
                source_name: source.to_string(),
                reason: database::redact(&e.to_string(), target),
            })?;

        // SQLite opens its file lazily, so a missing file can fail here rather than in ATTACH.
        let tables = database::list_tables(&self.conn, &catalog).map_err(|e| match e {
            EngineError::Duckdb(err) => EngineError::Attach {
                source_name: source.to_string(),
                reason: database::redact(&err.to_string(), target),
            },
            other => other,
        })?;
        self.conn
            .execute_batch(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))?;
        let mut views = HashSet::new();
        for (table_schema, table) in &tables {
            // The bare table name is used unless another schema has a table of the same name.
            let clashes = tables.iter().filter(|(_, t)| t == table).count() > 1;
            let view = if clashes {
                format!("{table_schema}_{table}")
            } else {
                table.clone()
            };
            if !views.insert(view.clone()) {
                return Err(EngineError::InvalidSpec(format!(
                    "database source '{source}' has tables that map to the same view name '{view}'"
                )));
            }
            self.conn.execute_batch(&format!(
                "CREATE OR REPLACE VIEW {schema}.{} AS SELECT * FROM {alias}.{}.{}",
                sql::quote_ident(&view),
                sql::quote_ident(table_schema),
                sql::quote_ident(table)
            ))?;
        }
        self.database_sources
            .borrow_mut()
            .insert(source.to_string());
        Ok(())
    }

    /// Loads one of the fixed DuckDB extensions, installing it on first use.
    fn load_extension(&self, name: &'static str) -> Result<(), EngineError> {
        if self.extensions.borrow().contains(name) {
            return Ok(());
        }
        // LOAD first, so an extension that is already installed works offline.
        let loaded = self
            .conn
            .execute_batch(&format!("LOAD {name}"))
            .or_else(|_| {
                self.conn
                    .execute_batch(&format!("INSTALL {name}; LOAD {name};"))
            });
        loaded.map_err(|e| EngineError::Extension {
            name: name.to_string(),
            reason: e.to_string(),
        })?;
        self.extensions.borrow_mut().insert(name);
        Ok(())
    }

    /// Creates a view named after `dataset`: its SQL, or `SELECT *` from its source,
    /// plus its calculated fields.
    ///
    /// Calculated fields are layered in declaration order: field N is computed over
    /// the base relation and fields 1 to N-1, so a field may use earlier ones but not
    /// later ones. Each expression is checked against the allowlist in [`calc`]; a
    /// field may not reuse the name of a column.
    pub fn register_dataset(&self, dataset: &Dataset) -> Result<(), EngineError> {
        dataset
            .validate_calculated_fields()
            .map_err(|e| EngineError::InvalidSpec(e.to_string()))?;
        let base = match &dataset.sql {
            Some(sql) => sql.clone(),
            None if self.database_sources.borrow().contains(&dataset.source) => {
                return Err(EngineError::InvalidSpec(format!(
                    "dataset '{}' needs an sql query: database source '{}' exposes several tables",
                    dataset.name, dataset.source
                )));
            }
            None => format!("SELECT * FROM {}", sql::quote_ident(&dataset.source)),
        };
        let body = if dataset.calculated_fields.is_empty() {
            base
        } else {
            self.with_calculated_fields(dataset, &base)?
        };
        let sql = format!(
            "CREATE OR REPLACE VIEW {} AS {body}",
            sql::quote_ident(&dataset.name)
        );
        self.conn.execute_batch(&sql)?;

        let names = dataset
            .calculated_fields
            .iter()
            .map(|f| f.name.to_ascii_lowercase())
            .collect();
        self.calculated
            .borrow_mut()
            .insert(dataset.name.clone(), names);
        Ok(())
    }

    /// Wraps `base` in one `SELECT *, (expr) AS "name"` layer per calculated field.
    /// Each layer is described by DuckDB, which reports bad references per field.
    fn with_calculated_fields(&self, dataset: &Dataset, base: &str) -> Result<String, EngineError> {
        let mut taken: HashSet<String> = self
            .column_names(base)?
            .into_iter()
            .map(|c| c.to_ascii_lowercase())
            .collect();
        let mut body = base.to_string();
        for field in &dataset.calculated_fields {
            let key = field.name.to_ascii_lowercase();
            if taken.contains(&key) {
                return Err(EngineError::InvalidSpec(format!(
                    "calculated field '{}' has the same name as a column of dataset '{}'",
                    field.name, dataset.name
                )));
            }
            calc::check(&self.conn, &field.name, &field.expression)?;
            let layer = format!(
                "SELECT *, ({}) AS {} FROM ({body})",
                field.expression,
                sql::quote_ident(&field.name)
            );
            self.column_names(&layer).map_err(|e| {
                EngineError::InvalidSpec(format!(
                    "calculated field '{}' cannot be evaluated on dataset '{}': {e}",
                    field.name, dataset.name
                ))
            })?;
            taken.insert(key);
            body = layer;
        }
        Ok(body)
    }

    /// The column names of a query, as DuckDB describes it (no rows are read).
    fn column_names(&self, select: &str) -> Result<Vec<String>, EngineError> {
        let batches = self.query(&format!("DESCRIBE {select}"))?;
        let mut names = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                names.push(cell(batch, "column_name", row)?);
            }
        }
        Ok(names)
    }

    /// Defines a zero-argument macro `"param_<name>"()` for each parameter that has a
    /// value in `values`, so SQL run against a report can use the bound value. The
    /// value is rendered by the same typed-literal path as filter values.
    pub fn define_parameter_macros(
        &self,
        parameters: &[Parameter],
        values: &ParamValues,
    ) -> Result<(), EngineError> {
        for parameter in parameters {
            let Some(value) = values.get(&parameter.name) else {
                continue;
            };
            let literal = sql::literal(value)?;
            let body = if parameter.param_type() == ParamType::Date {
                format!("CAST({literal} AS DATE)")
            } else {
                literal
            };
            self.conn.execute_batch(&format!(
                "CREATE OR REPLACE MACRO {}() AS {body}",
                sql::quote_ident(&format!("param_{}", parameter.name))
            ))?;
        }
        Ok(())
    }

    /// Registers a bare file as a view named `name`, inferring the format from its extension.
    pub fn register_file(&self, name: &str, path: &Path) -> Result<FileFormat, EngineError> {
        let format = infer_format(path)?;
        let source = Source {
            name: name.to_string(),
            kind: SourceKind::File {
                path: PathBuf::from(path),
                format,
            },
        };
        self.register_source(&source, Path::new(""))?;
        Ok(format)
    }

    /// Returns the columns of a registered relation.
    pub fn schema(&self, relation: &str) -> Result<Vec<ColumnInfo>, EngineError> {
        let batches = self.query(&format!("DESCRIBE {}", sql::quote_ident(relation)))?;
        let calculated = self
            .calculated
            .borrow()
            .get(relation)
            .cloned()
            .unwrap_or_default();
        let mut columns = Vec::new();
        for batch in &batches {
            for row in 0..batch.num_rows() {
                let name = cell(batch, "column_name", row)?;
                columns.push(ColumnInfo {
                    calculated: calculated.contains(&name.to_ascii_lowercase()),
                    name,
                    data_type: cell(batch, "column_type", row)?,
                    nullable: cell(batch, "null", row)? == "YES",
                });
            }
        }
        Ok(columns)
    }

    /// Runs DuckDB's `SUMMARIZE` over a relation (per-column statistics).
    pub fn profile(&self, relation: &str) -> Result<Vec<RecordBatch>, EngineError> {
        self.query(&format!("SUMMARIZE {}", sql::quote_ident(relation)))
    }

    /// Runs a SQL query and collects the results as Arrow record batches.
    pub fn query(&self, sql: &str) -> Result<Vec<RecordBatch>, EngineError> {
        let mut stmt = self.conn.prepare(sql)?;
        let batches: Vec<RecordBatch> = stmt.query_arrow([])?.collect();
        Ok(batches)
    }

    /// Compiles a visual against its dataset's columns. The visual must already have
    /// its parameters bound (see `dv_core::bind_visual`).
    pub fn compile(&self, visual: &Visual) -> Result<String, EngineError> {
        let columns: Vec<String> = self
            .schema(&visual.dataset)?
            .into_iter()
            .map(|c| c.name)
            .collect();
        compile_visual_with(visual, Some(&columns))
    }

    /// Compiles and runs a visual's query.
    pub fn run_visual(&self, visual: &Visual) -> Result<Vec<RecordBatch>, EngineError> {
        self.query(&self.compile(visual)?)
    }

    /// Handle that interrupts the query currently running on this engine. It can be
    /// used from other threads while [`Engine::query`] holds the engine.
    pub fn interrupt_handle(&self) -> std::sync::Arc<duckdb::InterruptHandle> {
        self.conn.interrupt_handle()
    }

    /// Executes a statement that returns no rows (DDL, COPY, ...). Returns the affected-row count.
    pub fn execute(&self, sql: &str) -> Result<usize, EngineError> {
        Ok(self.conn.execute(sql, [])?)
    }

    /// Writes the result of `sql` to `path`, streaming inside DuckDB. Returns the row count.
    pub fn export(
        &self,
        sql: &str,
        path: &Path,
        format: OutputFormat,
    ) -> Result<usize, EngineError> {
        let options = match format {
            OutputFormat::Csv => "FORMAT CSV, HEADER",
            OutputFormat::Parquet => "FORMAT PARQUET",
        };
        let copy = format!(
            "COPY ({sql}) TO {} ({options})",
            sql::quote_literal(&path.to_string_lossy())
        );
        self.execute(&copy)
    }
}

fn reader_function(format: FileFormat) -> Result<&'static str, EngineError> {
    match format {
        FileFormat::Csv => Ok("read_csv_auto"),
        FileFormat::Parquet => Ok("read_parquet"),
        FileFormat::Json => Ok("read_json_auto"),
        FileFormat::Arrow => Err(EngineError::Unsupported(
            "Arrow IPC files are registered by Engine::register_arrow_file".into(),
        )),
    }
}

/// Infers a file format from its extension (case-insensitive).
pub fn infer_format(path: &Path) -> Result<FileFormat, EngineError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "csv" => Ok(FileFormat::Csv),
        "parquet" | "pq" => Ok(FileFormat::Parquet),
        "json" | "jsonl" | "ndjson" => Ok(FileFormat::Json),
        "arrow" | "feather" | "ipc" => Ok(FileFormat::Arrow),
        _ => Err(EngineError::Unsupported(format!(
            "cannot infer file format from '{}'",
            path.display()
        ))),
    }
}

/// Renders one cell of a batch as text, by column name.
fn cell(batch: &RecordBatch, column: &str, row: usize) -> Result<String, EngineError> {
    let array = batch
        .column_by_name(column)
        .ok_or_else(|| EngineError::Unsupported(format!("missing column '{column}'")))?;
    if array.is_null(row) {
        return Ok(String::new());
    }
    array_value_to_string(array, row).map_err(|e| EngineError::Unsupported(e.to_string()))
}

#[cfg(test)]
mod tests;
