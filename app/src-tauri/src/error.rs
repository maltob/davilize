//! Error type returned to the frontend. Every failure becomes `{ kind, message }`.

use dv_core::CoreError;
use dv_engine::EngineError;
use dv_render::RenderError;
use serde::Serialize;

/// A user-facing error. `kind` lets the UI pick a wording or an icon.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError {
    /// One of: `cancelled`, `invalid`, `query`, `source`, `io`, `output`, `internal`.
    pub kind: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new("invalid", message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }

    pub fn cancelled() -> Self {
        Self::new("cancelled", "The query was cancelled.")
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// DuckDB reports an interrupted query with an `INTERRUPT` error.
fn is_interrupt(err: &duckdb::Error) -> bool {
    err.to_string().contains("INTERRUPT")
}

impl From<EngineError> for ApiError {
    fn from(err: EngineError) -> Self {
        let kind = match &err {
            EngineError::Duckdb(inner) if is_interrupt(inner) => return ApiError::cancelled(),
            EngineError::Duckdb(_) => "query",
            EngineError::InvalidSpec(_) | EngineError::Unsupported(_) => "invalid",
            EngineError::MissingConnection { .. }
            | EngineError::Attach { .. }
            | EngineError::Extension { .. } => "source",
            EngineError::Io { .. } => "io",
        };
        ApiError::new(kind, err.to_string())
    }
}

impl From<CoreError> for ApiError {
    fn from(err: CoreError) -> Self {
        let kind = match &err {
            CoreError::Io { .. } => "io",
            CoreError::Json(_) | CoreError::UnsupportedVersion { .. } | CoreError::Invalid(_) => {
                "invalid"
            }
        };
        ApiError::new(kind, err.to_string())
    }
}

impl From<RenderError> for ApiError {
    fn from(err: RenderError) -> Self {
        let kind = match &err {
            RenderError::Output(_) => "output",
            _ => "invalid",
        };
        ApiError::new(kind, err.to_string())
    }
}

impl From<std::io::Error> for ApiError {
    fn from(err: std::io::Error) -> Self {
        ApiError::new("io", err.to_string())
    }
}
