//! Core report model for Davilize.
//!
//! A report is a versioned JSON document (`.dvz`) describing data sources,
//! datasets (named SQL over a source, with optional calculated fields),
//! visuals (chart specs) and parameters. Reports never contain secrets: database
//! sources only reference a connection name that is resolved outside the report.

mod dashboard;
mod error;
mod model;
mod params;
mod schedule;

pub use dashboard::{apply_dashboard_filters, check_filter_shape};
pub use error::CoreError;
pub use model::{
    Aggregate, CalculatedField, ChartType, DASHBOARD_DEFAULT_COLUMNS, DASHBOARD_DEFAULT_ROW_HEIGHT,
    DASHBOARD_MAX_COLUMNS, DASHBOARD_MAX_ROW_HEIGHT, DASHBOARD_MIN_COLUMNS,
    DASHBOARD_MIN_ROW_HEIGHT, Dashboard, DashboardItem, Dataset, DbKind, Encodings, FileFormat,
    Filter, FilterOp, FilterScope, Join, JoinKey, JoinKind, Limit, Measure, ParamRef, ParamType,
    Parameter, REPORT_VERSION, Report, Sort, SortDirection, Source, SourceKind, Visual,
};
pub use params::{ParamValues, bind_visual, param_ref, parse_date, resolve_parameters};
pub use schedule::{
    PUBLISH_SCHEDULE_FORMATS, SCHEDULE_FORMATS, Schedule, ScheduleAction, ScheduleParam,
    check_publish_target, describe_cron,
};
