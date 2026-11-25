//! SQL compilation lives in the `dv-sql` crate (so the browser build can use it). This
//! module keeps the engine's error type on the same function names.

use dv_core::Visual;
use serde_json::Value;

use crate::EngineError;

pub use dv_sql::{quote_ident, quote_literal};

pub fn literal(value: &Value) -> Result<String, EngineError> {
    Ok(dv_sql::literal(value)?)
}

pub fn compile_visual(visual: &Visual) -> Result<String, EngineError> {
    Ok(dv_sql::compile_visual(visual)?)
}

pub fn compile_visual_with(
    visual: &Visual,
    columns: Option<&[String]>,
) -> Result<String, EngineError> {
    Ok(dv_sql::compile_visual_with(visual, columns)?)
}
