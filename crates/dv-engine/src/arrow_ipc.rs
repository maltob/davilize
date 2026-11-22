//! Arrow IPC file sources (`.arrow`, `.feather`).
//!
//! The file is read with the `arrow` crate's IPC `FileReader` and every batch is
//! copied into a DuckDB table through DuckDB's `arrow(?)` table function, which
//! is part of the bundled `vtab-arrow` support. No DuckDB extension download is
//! needed, so this works offline. The data is held in memory for the session.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use duckdb::arrow::ipc::reader::FileReader;
use duckdb::arrow::record_batch::RecordBatch;
use duckdb::vtab::arrow::ArrowBatchRegistration;

use crate::{Engine, EngineError, sql};

/// Schema that holds the materialized Arrow tables, kept apart from user views.
const ARROW_SCHEMA: &str = "dv_arrow";

impl Engine {
    /// Loads an Arrow IPC file into `dv_arrow."<name>"` and creates a view `"<name>"` over it.
    pub(crate) fn register_arrow_file(&self, name: &str, path: &Path) -> Result<(), EngineError> {
        let file = File::open(path).map_err(|source| EngineError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = FileReader::try_new(BufReader::new(file), None).map_err(|e| {
            EngineError::Unsupported(format!(
                "'{}' is not a readable Arrow IPC file: {e}",
                path.display()
            ))
        })?;
        let schema = reader.schema();

        let table = format!("{ARROW_SCHEMA}.{}", sql::quote_ident(name));
        self.conn
            .execute_batch(&format!("CREATE SCHEMA IF NOT EXISTS {ARROW_SCHEMA}"))?;
        // An empty batch with the file's schema gives the table its column types.
        let empty = RecordBatch::new_empty(schema);
        self.conn.execute(
            &format!("CREATE OR REPLACE TABLE {table} AS SELECT * FROM arrow(?)"),
            [&ArrowBatchRegistration::new(empty)],
        )?;

        let insert = format!("INSERT INTO {table} SELECT * FROM arrow(?)");
        for batch in reader {
            let batch = batch.map_err(|e| {
                EngineError::Unsupported(format!(
                    "'{}' has an unreadable Arrow batch: {e}",
                    path.display()
                ))
            })?;
            self.conn
                .execute(&insert, [&ArrowBatchRegistration::new(batch)])?;
        }

        self.conn.execute_batch(&format!(
            "CREATE OR REPLACE VIEW {} AS SELECT * FROM {table}",
            sql::quote_ident(name)
        ))?;
        Ok(())
    }
}
