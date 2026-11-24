//! Dashboard filters: the shape check for a filter's operator and values, and the
//! rule that gives each visual on a dashboard its shared filters.
//!
//! Pure: no DuckDB and no files, so the GUI can call the same functions.

use serde_json::Value;

use crate::CoreError;
use crate::model::{Dashboard, Filter, FilterOp, FilterScope, Visual};
use crate::params::{ParamValues, param_ref};

/// Checks that a filter's values fit its operator, as the engine requires when it
/// compiles the filter: `eq`/`ne` take one value, the comparisons take one non-null
/// value, `in` takes non-null values, `between` takes two non-null values (lower,
/// then upper), and `is_null`/`is_not_null` take none. Values are scalars or
/// `{"param": ...}` references (the values of a reference are checked at refresh).
pub fn check_filter_shape(filter: &Filter) -> Result<(), String> {
    let field = filter.field.as_str();
    if field.is_empty() {
        return Err("a filter needs a field name".to_string());
    }
    for value in &filter.values {
        let is_reference = param_ref(value).map_err(|e| e.to_string())?.is_some();
        if !is_reference && matches!(value, Value::Array(_) | Value::Object(_)) {
            return Err(format!(
                "filter on '{field}': values must be scalars or parameter references"
            ));
        }
    }

    let values = filter.values.as_slice();
    let no_nulls = || values.iter().all(|v| !v.is_null());
    let (ok, need) = match filter.op {
        FilterOp::Eq | FilterOp::Ne => (values.len() == 1, "exactly one value"),
        FilterOp::Lt | FilterOp::Le | FilterOp::Gt | FilterOp::Ge => (
            values.len() == 1 && no_nulls(),
            "exactly one non-null value",
        ),
        FilterOp::In => (
            !values.is_empty() && no_nulls(),
            "at least one non-null value",
        ),
        FilterOp::Between => (
            values.len() == 2 && no_nulls(),
            "exactly two non-null values: lower, then upper",
        ),
        FilterOp::IsNull | FilterOp::IsNotNull => (values.is_empty(), "no values"),
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "filter on '{field}' ({}) needs {need}",
            op_name(filter.op)
        ))
    }
}

fn op_name(op: FilterOp) -> &'static str {
    match op {
        FilterOp::Eq => "eq",
        FilterOp::Ne => "ne",
        FilterOp::Lt => "lt",
        FilterOp::Le => "le",
        FilterOp::Gt => "gt",
        FilterOp::Ge => "ge",
        FilterOp::In => "in",
        FilterOp::Between => "between",
        FilterOp::IsNull => "is_null",
        FilterOp::IsNotNull => "is_not_null",
    }
}

/// Returns `visual` with the dashboard's shared filters appended to its own.
///
/// Rules:
/// * Only row filters are shared. An aggregate-scope filter is an error.
/// * A shared filter applies only when `columns` (the visual's dataset columns,
///   including calculated fields) contains its `field`, matched exactly. A visual
///   whose dataset lacks the column is returned without that filter.
/// * `{"param": ...}` values in a shared filter are bound from `values`.
///
/// `visual` should already have its parameters bound (see [`crate::bind_visual`]),
/// and the result is what gets compiled and rendered for that placement. Parameters
/// are global to the report, so `values` are the report's bound parameter values.
pub fn apply_dashboard_filters(
    dashboard: &Dashboard,
    visual: &Visual,
    columns: &[String],
    values: &ParamValues,
) -> Result<Visual, CoreError> {
    let mut shared = visual.clone();
    for filter in &dashboard.filters {
        if filter.scope == FilterScope::Aggregate {
            return Err(CoreError::Invalid(format!(
                "dashboard '{}': filter on '{}' uses scope 'aggregate'; dashboard filters apply to rows only",
                dashboard.id, filter.field
            )));
        }
        if !columns.iter().any(|column| column == &filter.field) {
            continue;
        }
        let mut bound = filter.clone();
        for value in &mut bound.values {
            if let Some(name) = param_ref(value)?.map(str::to_owned) {
                *value = values
                    .get(&name)
                    .cloned()
                    .ok_or_else(|| CoreError::Invalid(format!("unknown parameter '{name}'")))?;
            }
        }
        shared.filters.push(bound);
    }
    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        ChartType, DashboardItem, Dataset, Encodings, FileFormat, Parameter, Report, Source,
        SourceKind,
    };
    use serde_json::json;

    fn filter(field: &str, op: FilterOp, values: Vec<Value>) -> Filter {
        Filter {
            field: field.into(),
            op,
            values,
            scope: FilterScope::Row,
        }
    }

    fn visual(id: &str) -> Visual {
        Visual {
            id: id.into(),
            title: id.into(),
            dataset: "sales".into(),
            chart: ChartType::Table,
            encodings: Encodings::default(),
            dimensions: vec!["product".into()],
            measures: vec![],
            filters: vec![],
            sort: None,
            limit: None,
        }
    }

    fn columns(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn shapes_follow_the_engine_rules() {
        let ok = [
            filter("a", FilterOp::Eq, vec![json!("x")]),
            filter("a", FilterOp::Eq, vec![Value::Null]),
            filter("a", FilterOp::Ne, vec![json!({"param": "p"})]),
            filter("a", FilterOp::Gt, vec![json!(1)]),
            filter("a", FilterOp::In, vec![json!("x"), json!("y")]),
            filter("a", FilterOp::Between, vec![json!(1), json!(9)]),
            filter("a", FilterOp::IsNull, vec![]),
            filter("a", FilterOp::IsNotNull, vec![]),
        ];
        for f in &ok {
            assert!(check_filter_shape(f).is_ok(), "{f:?}");
        }
        let bad = [
            filter("", FilterOp::Eq, vec![json!("x")]),
            filter("a", FilterOp::Eq, vec![]),
            filter("a", FilterOp::Eq, vec![json!(1), json!(2)]),
            filter("a", FilterOp::Gt, vec![Value::Null]),
            filter("a", FilterOp::In, vec![]),
            filter("a", FilterOp::In, vec![json!("x"), Value::Null]),
            filter("a", FilterOp::Between, vec![json!(1)]),
            filter("a", FilterOp::Between, vec![json!(1), Value::Null]),
            filter("a", FilterOp::IsNull, vec![json!(1)]),
            filter("a", FilterOp::Eq, vec![json!([1, 2])]),
            filter("a", FilterOp::Eq, vec![json!({"x": 1})]),
        ];
        for f in &bad {
            assert!(check_filter_shape(f).is_err(), "{f:?}");
        }
    }

    #[test]
    fn shared_filter_applies_only_to_visuals_with_the_column() {
        let mut dashboard = Dashboard::new("d", "D");
        dashboard.filters = vec![
            filter("region", FilterOp::Eq, vec![json!({"param": "region"})]),
            filter("units", FilterOp::Ge, vec![json!(2)]),
        ];
        let mut values = ParamValues::new();
        values.insert("region".into(), json!("North"));

        // The dataset has both columns: both filters are appended, values bound.
        let cols = columns(&["region", "product", "units"]);
        let out = apply_dashboard_filters(&dashboard, &visual("v"), &cols, &values).unwrap();
        assert_eq!(out.filters.len(), 2);
        assert_eq!(out.filters[0].values, vec![json!("North")]);
        assert_eq!(out.filters[1].field, "units");

        // No region column: only the units filter is appended.
        let cols = columns(&["product", "units"]);
        let out = apply_dashboard_filters(&dashboard, &visual("v"), &cols, &values).unwrap();
        assert_eq!(out.filters.len(), 1);
        assert_eq!(out.filters[0].field, "units");

        // The visual's own filters stay first.
        let mut own = visual("v");
        own.filters = vec![filter("product", FilterOp::Eq, vec![json!("Widget")])];
        let out = apply_dashboard_filters(&dashboard, &own, &cols, &values).unwrap();
        assert_eq!(out.filters[0].field, "product");
        assert_eq!(out.filters.len(), 2);
    }

    #[test]
    fn shared_filter_with_unbound_parameter_is_an_error() {
        let mut dashboard = Dashboard::new("d", "D");
        dashboard.filters = vec![filter(
            "region",
            FilterOp::Eq,
            vec![json!({"param": "nope"})],
        )];
        let err = apply_dashboard_filters(
            &dashboard,
            &visual("v"),
            &columns(&["region"]),
            &ParamValues::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown parameter 'nope'"));
    }

    #[test]
    fn shared_aggregate_filter_is_refused() {
        let mut dashboard = Dashboard::new("d", "D");
        dashboard.filters = vec![Filter {
            field: "total".into(),
            op: FilterOp::Gt,
            values: vec![json!(1)],
            scope: FilterScope::Aggregate,
        }];
        assert!(
            apply_dashboard_filters(
                &dashboard,
                &visual("v"),
                &columns(&["total"]),
                &ParamValues::new()
            )
            .is_err()
        );
    }

    /// A report with one visual and one dashboard, for validation tests.
    fn report_with(dashboard: Dashboard) -> Report {
        let mut report = Report::new("r");
        report.sources = vec![Source {
            name: "s".into(),
            kind: SourceKind::File {
                path: "s.csv".into(),
                format: FileFormat::Csv,
            },
        }];
        report.datasets = vec![Dataset {
            name: "sales".into(),
            source: "s".into(),
            sql: None,
            joins: Vec::new(),
            calculated_fields: vec![],
        }];
        report.parameters = vec![Parameter::new("region", Some(json!("North")))];
        report.visuals = vec![visual("v1"), visual("v2")];
        report.dashboards = vec![dashboard];
        report
    }

    fn item(visual: &str, x: u32, y: u32, w: u32, h: u32) -> DashboardItem {
        DashboardItem {
            visual: visual.into(),
            x,
            y,
            w,
            h,
        }
    }

    #[test]
    fn dashboard_placements_are_checked() {
        let base = || {
            let mut d = Dashboard::new("d", "D");
            d.items = vec![item("v1", 0, 0, 6, 2), item("v2", 6, 0, 6, 2)];
            d
        };
        assert!(report_with(base()).validate().is_ok());

        let cases: Vec<(&str, Dashboard)> = vec![
            ("unknown visual", {
                let mut d = base();
                d.items[0].visual = "missing".into();
                d
            }),
            ("placed twice", {
                let mut d = base();
                d.items.push(item("v1", 0, 4, 1, 1));
                d
            }),
            ("zero width", {
                let mut d = base();
                d.items[0].w = 0;
                d
            }),
            ("zero height", {
                let mut d = base();
                d.items[0].h = 0;
                d
            }),
            ("past the last column", {
                let mut d = base();
                d.items[1].x = 7;
                d
            }),
            ("overlap", {
                let mut d = base();
                d.items[1].x = 5;
                d
            }),
            ("columns too small", {
                let mut d = base();
                d.columns = 0;
                d
            }),
            ("columns too large", {
                let mut d = base();
                d.columns = 25;
                d
            }),
            ("row height too small", {
                let mut d = base();
                d.row_height = 19;
                d
            }),
            ("row height too large", {
                let mut d = base();
                d.row_height = 401;
                d
            }),
            ("bad dashboard id", {
                let mut d = base();
                d.id = "../x".into();
                d
            }),
            ("aggregate filter", {
                let mut d = base();
                d.filters = vec![Filter {
                    field: "total".into(),
                    op: FilterOp::Gt,
                    values: vec![json!(1)],
                    scope: FilterScope::Aggregate,
                }];
                d
            }),
            ("filter shape", {
                let mut d = base();
                d.filters = vec![filter("region", FilterOp::Between, vec![json!("A")])];
                d
            }),
            ("undeclared parameter", {
                let mut d = base();
                d.filters = vec![filter(
                    "region",
                    FilterOp::Eq,
                    vec![json!({"param": "nope"})],
                )];
                d
            }),
        ];
        for (case, dashboard) in cases {
            let err = report_with(dashboard).validate();
            assert!(matches!(err, Err(CoreError::Invalid(_))), "{case}: {err:?}");
        }

        // Touching edges do not overlap; a full-width item is allowed.
        let mut d = Dashboard::new("d", "D");
        d.items = vec![item("v1", 0, 0, 12, 2), item("v2", 0, 2, 12, 2)];
        assert!(report_with(d).validate().is_ok());
    }

    #[test]
    fn duplicate_dashboard_ids_are_rejected() {
        let mut report = report_with(Dashboard::new("d", "D"));
        report.dashboards.push(Dashboard::new("d", "Again"));
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));
    }
}
