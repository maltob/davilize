use std::path::PathBuf;

/// Errors produced when reading, writing or validating a report.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid report JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("unsupported report version {found} (this build supports {supported})")]
    UnsupportedVersion { found: u32, supported: u32 },

    #[error("invalid report: {0}")]
    Invalid(String),
}
