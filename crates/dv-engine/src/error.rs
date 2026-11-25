use std::path::PathBuf;

/// Errors produced by the query engine.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    /// A DuckDB error (SQL syntax, missing column, bad file contents, ...).
    #[error("database error: {0}")]
    Duckdb(#[from] duckdb::Error),

    /// A visual or query spec that cannot be compiled.
    #[error("invalid spec: {0}")]
    InvalidSpec(String),

    /// A feature that is not implemented yet, or an unsupported input.
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// A database source whose connection reference has no value in the environment.
    #[error(
        "database connection '{connection}' is not configured: set the environment variable {env_var}"
    )]
    MissingConnection { connection: String, env_var: String },

    /// A database source that DuckDB could not attach (secrets are redacted from the reason).
    #[error("cannot attach database source '{source_name}': {reason}")]
    Attach { source_name: String, reason: String },

    /// A DuckDB extension that could not be loaded (first use downloads it).
    #[error(
        "DuckDB extension '{name}' could not be loaded (the first use downloads it and needs network access): {reason}"
    )]
    Extension { name: String, reason: String },

    /// A file that is missing or cannot be accessed.
    #[error("I/O error on {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl From<dv_sql::SqlError> for EngineError {
    fn from(err: dv_sql::SqlError) -> Self {
        match err {
            dv_sql::SqlError::InvalidSpec(message) => EngineError::InvalidSpec(message),
        }
    }
}
