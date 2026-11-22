//! Core report model for Davilize.
//!
//! A report is a versioned JSON document (`.dvz`) describing data sources,
//! datasets (named SQL over a source, with optional calculated fields),
//! visuals (chart specs) and parameters. Reports never contain secrets: database
//! sources only reference a connection name that is resolved outside the report.

mod error;
mod model;
mod params;

pub use error::CoreError;
pub use model::{
    Aggregate, CalculatedField, ChartType, Dataset, DbKind, Encodings, FileFormat, Filter,
    FilterOp, FilterScope, Limit, Measure, ParamRef, ParamType, Parameter, REPORT_VERSION, Report,
    Sort, SortDirection, Source, SourceKind, Visual,
};
pub use params::{ParamValues, bind_visual, param_ref, parse_date, resolve_parameters};
