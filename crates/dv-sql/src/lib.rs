//! Compiles a [`Visual`] spec into DuckDB SQL.
//!
//! Every identifier is quoted with `"` (embedded quotes doubled) and every
//! literal is rendered by [`quote_literal`] (embedded quotes doubled, numbers
//! validated as finite). Field and alias names never reach the SQL unquoted,
//! and filter values are typed JSON scalars, so spec content cannot inject SQL.
//!
//! Filters: `row` filters (the default) become WHERE predicates over dataset
//! columns; `aggregate` filters become HAVING predicates over the aggregate
//! expression of a measure (not its alias, to stay portable).

use std::collections::HashSet;

use dv_core::{Aggregate, Filter, FilterOp, FilterScope, Limit, Measure, SortDirection, Visual};
use serde_json::Value;

/// Errors from compiling a spec to SQL.
#[derive(Debug, thiserror::Error)]
pub enum SqlError {
    /// A visual or query spec that cannot be compiled.
    #[error("invalid spec: {0}")]
    InvalidSpec(String),
}

/// Quotes an identifier (column, view or alias) for DuckDB.
pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Quotes a string as a SQL string literal.
pub fn quote_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// Renders a JSON scalar as a SQL literal. Arrays and objects are rejected.
pub fn literal(value: &Value) -> Result<String, SqlError> {
    match value {
        Value::Null => Ok("NULL".to_string()),
        Value::Bool(true) => Ok("TRUE".to_string()),
        Value::Bool(false) => Ok("FALSE".to_string()),
        Value::String(s) => Ok(quote_literal(s)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if let Some(f) = n.as_f64().filter(|f| f.is_finite()) {
                Ok(format!("{f:?}"))
            } else {
                Err(SqlError::InvalidSpec(format!("unsupported number {n}")))
            }
        }
        other => Err(SqlError::InvalidSpec(format!(
            "filter values must be scalars, got {other}"
        ))),
    }
}

fn check_field(name: &str) -> Result<(), SqlError> {
    if name.is_empty() || name.contains('\0') {
        return Err(SqlError::InvalidSpec(format!(
            "invalid field name {name:?}"
        )));
    }
    Ok(())
}

fn aggregate_expr(measure: &Measure) -> String {
    let field = quote_ident(&measure.field);
    match measure.agg {
        Aggregate::Sum => format!("SUM({field})"),
        Aggregate::Avg => format!("AVG({field})"),
        Aggregate::Min => format!("MIN({field})"),
        Aggregate::Max => format!("MAX({field})"),
        Aggregate::Count => format!("COUNT({field})"),
        Aggregate::CountDistinct => format!("COUNT(DISTINCT {field})"),
    }
}

/// Parameter references must be bound before compiling. Reaching this point
/// with one means the caller skipped binding, so it is an error, not a value.
fn reject_unbound(filter: &Filter) -> Result<(), SqlError> {
    for value in &filter.values {
        let reference =
            dv_core::param_ref(value).map_err(|e| SqlError::InvalidSpec(e.to_string()))?;
        if let Some(name) = reference {
            return Err(SqlError::InvalidSpec(format!(
                "filter on '{}' uses parameter '{name}', which has no bound value",
                filter.field
            )));
        }
    }
    Ok(())
}

/// Renders one filter as a predicate over `target`: the quoted column for row
/// filters, or the measure's aggregate expression for aggregate filters.
fn filter_predicate(filter: &Filter, target: &str) -> Result<String, SqlError> {
    check_field(&filter.field)?;
    reject_unbound(filter)?;
    let values = &filter.values;
    let single = |op: &str| -> Result<String, SqlError> {
        match values.as_slice() {
            [v] if !v.is_null() => Ok(format!("{target} {op} {}", literal(v)?)),
            _ => Err(SqlError::InvalidSpec(format!(
                "filter on '{}' needs exactly one non-null value",
                filter.field
            ))),
        }
    };
    let expect_none = |sql: String| -> Result<String, SqlError> {
        if values.is_empty() {
            Ok(sql)
        } else {
            Err(SqlError::InvalidSpec(format!(
                "filter on '{}' takes no values",
                filter.field
            )))
        }
    };
    match filter.op {
        FilterOp::Eq => match values.as_slice() {
            [Value::Null] => Ok(format!("{target} IS NULL")),
            _ => single("="),
        },
        FilterOp::Ne => match values.as_slice() {
            [Value::Null] => Ok(format!("{target} IS NOT NULL")),
            _ => single("<>"),
        },
        FilterOp::Lt => single("<"),
        FilterOp::Le => single("<="),
        FilterOp::Gt => single(">"),
        FilterOp::Ge => single(">="),
        FilterOp::In => {
            if values.is_empty() {
                return Err(SqlError::InvalidSpec(format!(
                    "'in' filter on '{}' needs at least one value",
                    filter.field
                )));
            }
            if values.iter().any(Value::is_null) {
                return Err(SqlError::InvalidSpec(format!(
                    "'in' filter on '{}' cannot contain null; use is_null",
                    filter.field
                )));
            }
            let items = values.iter().map(literal).collect::<Result<Vec<_>, _>>()?;
            Ok(format!("{target} IN ({})", items.join(", ")))
        }
        FilterOp::Between => match values.as_slice() {
            [lower, upper] if !lower.is_null() && !upper.is_null() => Ok(format!(
                "{target} BETWEEN {} AND {}",
                literal(lower)?,
                literal(upper)?
            )),
            _ => Err(SqlError::InvalidSpec(format!(
                "'between' filter on '{}' needs exactly two non-null values: lower, then upper",
                filter.field
            ))),
        },
        FilterOp::IsNull => expect_none(format!("{target} IS NULL")),
        FilterOp::IsNotNull => expect_none(format!("{target} IS NOT NULL")),
    }
}

/// A row filter must name a dataset column. When the columns are not known, it
/// may not name a measure output, since that alias is not a column.
fn check_row_filter(
    filter: &Filter,
    measure_outputs: &[String],
    columns: Option<&[String]>,
) -> Result<(), SqlError> {
    match columns {
        Some(columns) => {
            if columns
                .iter()
                .any(|c| c.eq_ignore_ascii_case(&filter.field))
            {
                Ok(())
            } else {
                Err(SqlError::InvalidSpec(format!(
                    "row filter on '{}' is not a column of the dataset; to filter a measure use scope 'aggregate'",
                    filter.field
                )))
            }
        }
        None if measure_outputs.contains(&filter.field) => Err(SqlError::InvalidSpec(format!(
            "row filter on '{}' names a measure output; use scope 'aggregate' to filter on a measure",
            filter.field
        ))),
        None => Ok(()),
    }
}

/// Compiles a visual into a single `SELECT` over its dataset, without knowing
/// the dataset's columns. See [`compile_visual_with`].
pub fn compile_visual(visual: &Visual) -> Result<String, SqlError> {
    compile_visual_with(visual, None)
}

/// Compiles a visual into a single `SELECT` over its dataset.
///
/// * With measures: `GROUP BY dimensions`, output = dimensions + measure aliases.
/// * Without measures but with dimensions: `SELECT DISTINCT dimensions`.
/// * Without either: raw rows, projected to the encoding fields (or `*`).
///
/// `columns` are the dataset's column names when the caller knows them (the engine
/// does). With them, row filters must name a real column; without them, a row
/// filter that names a measure output is rejected.
pub fn compile_visual_with(
    visual: &Visual,
    columns: Option<&[String]>,
) -> Result<String, SqlError> {
    check_field(&visual.dataset)?;
    let grouped = !visual.measures.is_empty();

    let mut outputs: Vec<String> = Vec::new();
    let mut select: Vec<String> = Vec::new();
    for dim in &visual.dimensions {
        check_field(dim)?;
        if outputs.contains(dim) {
            return Err(SqlError::InvalidSpec(format!(
                "duplicate dimension '{dim}'"
            )));
        }
        outputs.push(dim.clone());
        select.push(quote_ident(dim));
    }
    for measure in &visual.measures {
        check_field(&measure.field)?;
        let alias = measure.output_name();
        check_field(&alias)?;
        if outputs.contains(&alias) {
            return Err(SqlError::InvalidSpec(format!(
                "output name '{alias}' is used more than once"
            )));
        }
        select.push(format!(
            "{} AS {}",
            aggregate_expr(measure),
            quote_ident(&alias)
        ));
        outputs.push(alias);
    }

    // `available` is the set of names a visual may refer to in encodings and sort.
    // In raw mode the projection is derived from the encodings, so nothing is checked.
    let (select_sql, available): (String, Option<HashSet<String>>) = if grouped {
        (select.join(", "), Some(outputs.iter().cloned().collect()))
    } else if !visual.dimensions.is_empty() {
        (
            format!("DISTINCT {}", select.join(", ")),
            Some(outputs.iter().cloned().collect()),
        )
    } else {
        let mut raw: Vec<String> = Vec::new();
        for field in [
            &visual.encodings.x,
            &visual.encodings.y,
            &visual.encodings.color,
            &visual.encodings.size,
        ]
        .into_iter()
        .flatten()
        {
            check_field(field)?;
            if !raw.contains(field) {
                raw.push(field.clone());
            }
        }
        let sql = if raw.is_empty() {
            "*".to_string()
        } else {
            raw.iter()
                .map(|f| quote_ident(f))
                .collect::<Vec<_>>()
                .join(", ")
        };
        (sql, None)
    };

    if let Some(available) = &available {
        for (channel, field) in [
            ("x", &visual.encodings.x),
            ("y", &visual.encodings.y),
            ("color", &visual.encodings.color),
            ("size", &visual.encodings.size),
        ] {
            if let Some(field) = field
                && !available.contains(field)
            {
                return Err(SqlError::InvalidSpec(format!(
                    "encoding '{channel}' refers to unknown field '{field}'"
                )));
            }
        }
    }

    // Filters: row filters go to WHERE, aggregate filters to HAVING.
    let measure_outputs: Vec<String> = visual.measures.iter().map(Measure::output_name).collect();
    let mut where_terms: Vec<String> = Vec::new();
    let mut having_terms: Vec<String> = Vec::new();
    for filter in &visual.filters {
        match filter.scope {
            FilterScope::Row => {
                check_row_filter(filter, &measure_outputs, columns)?;
                where_terms.push(filter_predicate(filter, &quote_ident(&filter.field))?);
            }
            FilterScope::Aggregate => {
                if !grouped {
                    return Err(SqlError::InvalidSpec(format!(
                        "aggregate filter on '{}' needs at least one measure in the visual",
                        filter.field
                    )));
                }
                let measure = visual
                    .measures
                    .iter()
                    .find(|m| m.output_name() == filter.field)
                    .ok_or_else(|| {
                        SqlError::InvalidSpec(format!(
                            "aggregate filter field '{}' is not a measure output (outputs: {})",
                            filter.field,
                            measure_outputs.join(", ")
                        ))
                    })?;
                having_terms.push(filter_predicate(filter, &aggregate_expr(measure))?);
            }
        }
    }

    let mut sql = format!("SELECT {select_sql} FROM {}", quote_ident(&visual.dataset));

    if !where_terms.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_terms.join(" AND "));
    }

    if grouped && !visual.dimensions.is_empty() {
        let dims: Vec<String> = visual.dimensions.iter().map(|d| quote_ident(d)).collect();
        sql.push_str(" GROUP BY ");
        sql.push_str(&dims.join(", "));
    }

    if !having_terms.is_empty() {
        sql.push_str(" HAVING ");
        sql.push_str(&having_terms.join(" AND "));
    }

    // ORDER BY: the requested sort first, then the grouping dimensions as
    // tie-breakers. Without the tie-breakers, rows with equal sort values (or
    // all rows, when there is no sort) come back in an arbitrary order that can
    // differ between runs, which makes published reports flip rows around.
    let mut order_terms: Vec<String> = Vec::new();
    let mut sorted_by: Option<&str> = None;
    if let Some(sort) = &visual.sort {
        check_field(&sort.by)?;
        if let Some(available) = &available
            && !available.contains(&sort.by)
        {
            return Err(SqlError::InvalidSpec(format!(
                "sort refers to unknown field '{}'",
                sort.by
            )));
        }
        let direction = match sort.direction {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        };
        order_terms.push(format!("{} {direction}", quote_ident(&sort.by)));
        sorted_by = Some(sort.by.as_str());
    }
    if grouped {
        for dimension in &visual.dimensions {
            if sorted_by != Some(dimension.as_str()) {
                order_terms.push(format!("{} ASC", quote_ident(dimension)));
            }
        }
    }
    if !order_terms.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order_terms.join(", "));
    }

    match &visual.limit {
        None => {}
        Some(Limit::Rows(rows)) => sql.push_str(&format!(" LIMIT {rows}")),
        Some(Limit::Param(reference)) => {
            return Err(SqlError::InvalidSpec(format!(
                "limit parameter '{}' has no bound value",
                reference.param
            )));
        }
    }

    Ok(sql)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_core::{ChartType, Encodings, Sort};

    fn base(chart: ChartType) -> Visual {
        Visual {
            id: "v".into(),
            title: "t".into(),
            dataset: "sales".into(),
            chart,
            encodings: Encodings::default(),
            dimensions: vec![],
            measures: vec![],
            filters: vec![],
            sort: None,
            limit: None,
        }
    }

    fn sum(field: &str, alias: Option<&str>) -> Measure {
        Measure {
            field: field.into(),
            agg: Aggregate::Sum,
            alias: alias.map(String::from),
        }
    }

    #[test]
    fn grouped_bar_chart() {
        let mut v = base(ChartType::Bar);
        v.dimensions = vec!["region".into()];
        v.measures = vec![sum("revenue", Some("total"))];
        v.encodings.x = Some("region".into());
        v.encodings.y = Some("total".into());
        v.sort = Some(Sort {
            by: "total".into(),
            direction: SortDirection::Desc,
        });
        v.limit = Some(Limit::Rows(5));
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "region", SUM("revenue") AS "total" FROM "sales" GROUP BY "region" ORDER BY "total" DESC, "region" ASC LIMIT 5"#
        );
    }

    #[test]
    fn dimension_tie_breakers_skip_the_sort_column_and_keep_dimension_order() {
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into(), "product".into()];
        v.measures = vec![sum("revenue", Some("total"))];
        v.sort = Some(Sort {
            by: "region".into(),
            direction: SortDirection::Desc,
        });
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "region", "product", SUM("revenue") AS "total" FROM "sales" GROUP BY "region", "product" ORDER BY "region" DESC, "product" ASC"#
        );
        // Raw (ungrouped) rows have no natural key, so no ORDER BY is invented.
        let mut raw = base(ChartType::Scatter);
        raw.encodings.x = Some("units".into());
        raw.encodings.y = Some("revenue".into());
        assert!(!compile_visual(&raw).unwrap().contains("ORDER BY"));
    }

    #[test]
    fn default_measure_alias_and_count_distinct() {
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into()];
        v.measures = vec![Measure {
            field: "customer".into(),
            agg: Aggregate::CountDistinct,
            alias: None,
        }];
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "region", COUNT(DISTINCT "customer") AS "count_distinct_customer" FROM "sales" GROUP BY "region" ORDER BY "region" ASC"#
        );
    }

    #[test]
    fn all_aggregates_render() {
        let mut v = base(ChartType::Table);
        v.measures = vec![
            Measure {
                field: "a".into(),
                agg: Aggregate::Avg,
                alias: Some("x1".into()),
            },
            Measure {
                field: "a".into(),
                agg: Aggregate::Min,
                alias: Some("x2".into()),
            },
            Measure {
                field: "a".into(),
                agg: Aggregate::Max,
                alias: Some("x3".into()),
            },
            Measure {
                field: "a".into(),
                agg: Aggregate::Count,
                alias: Some("x4".into()),
            },
        ];
        let sql = compile_visual(&v).unwrap();
        assert!(sql.starts_with(
            r#"SELECT AVG("a") AS "x1", MIN("a") AS "x2", MAX("a") AS "x3", COUNT("a") AS "x4" FROM "sales""#
        ));
        assert!(!sql.contains("GROUP BY"));
    }

    #[test]
    fn dimensions_without_measures_are_distinct() {
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into(), "product".into()];
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT DISTINCT "region", "product" FROM "sales""#
        );
    }

    #[test]
    fn raw_scatter_projects_encoding_fields() {
        let mut v = base(ChartType::Scatter);
        v.encodings = Encodings {
            x: Some("units".into()),
            y: Some("revenue".into()),
            color: Some("region".into()),
            size: Some("units".into()),
        };
        v.limit = Some(Limit::Rows(1000));
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "units", "revenue", "region" FROM "sales" LIMIT 1000"#
        );
    }

    #[test]
    fn empty_spec_selects_everything() {
        let v = base(ChartType::Table);
        assert_eq!(compile_visual(&v).unwrap(), r#"SELECT * FROM "sales""#);
    }

    #[test]
    fn filters_render_with_typed_literals() {
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into()];
        v.filters = vec![
            Filter {
                field: "region".into(),
                op: FilterOp::In,
                values: vec![json("North"), json("South")],
                scope: FilterScope::Row,
            },
            Filter {
                field: "units".into(),
                op: FilterOp::Ge,
                values: vec![serde_json::json!(3)],
                scope: FilterScope::Row,
            },
            Filter {
                field: "price".into(),
                op: FilterOp::Lt,
                values: vec![serde_json::json!(9.5)],
                scope: FilterScope::Row,
            },
            Filter {
                field: "flag".into(),
                op: FilterOp::Eq,
                values: vec![serde_json::json!(true)],
                scope: FilterScope::Row,
            },
            Filter {
                field: "note".into(),
                op: FilterOp::IsNotNull,
                values: vec![],
                scope: FilterScope::Row,
            },
            Filter {
                field: "note".into(),
                op: FilterOp::Ne,
                values: vec![Value::Null],
                scope: FilterScope::Row,
            },
        ];
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT DISTINCT "region" FROM "sales" WHERE "region" IN ('North', 'South') AND "units" >= 3 AND "price" < 9.5 AND "flag" = TRUE AND "note" IS NOT NULL AND "note" IS NOT NULL"#
        );
    }

    #[test]
    fn eq_null_becomes_is_null() {
        let f = Filter {
            field: "note".into(),
            op: FilterOp::Eq,
            values: vec![Value::Null],
            scope: FilterScope::Row,
        };
        assert_eq!(
            filter_predicate(&f, r#""note""#).unwrap(),
            r#""note" IS NULL"#
        );
    }

    #[test]
    fn identifiers_and_literals_are_escaped() {
        let mut v = base(ChartType::Table);
        v.dataset = r#"sales"; DROP TABLE users; --"#.into();
        v.dimensions = vec![r#"we"ird"#.into()];
        v.filters = vec![Filter {
            field: "name".into(),
            op: FilterOp::Eq,
            values: vec![json("O'Brien'); DROP TABLE x; --")],
            scope: FilterScope::Row,
        }];
        let sql = compile_visual(&v).unwrap();
        assert_eq!(
            sql,
            r#"SELECT DISTINCT "we""ird" FROM "sales""; DROP TABLE users; --" WHERE "name" = 'O''Brien''); DROP TABLE x; --'"#
        );
    }

    #[test]
    fn rejects_unknown_encoding_and_sort_fields() {
        let mut v = base(ChartType::Bar);
        v.dimensions = vec!["region".into()];
        v.measures = vec![sum("revenue", None)];
        v.encodings.x = Some("product".into());
        assert!(matches!(compile_visual(&v), Err(SqlError::InvalidSpec(_))));

        let mut v = base(ChartType::Bar);
        v.dimensions = vec!["region".into()];
        v.measures = vec![sum("revenue", None)];
        v.sort = Some(Sort {
            by: "nope".into(),
            direction: SortDirection::Asc,
        });
        assert!(matches!(compile_visual(&v), Err(SqlError::InvalidSpec(_))));
    }

    #[test]
    fn rejects_bad_filters() {
        let cases = vec![
            Filter {
                field: "a".into(),
                op: FilterOp::Eq,
                values: vec![],
                scope: FilterScope::Row,
            },
            Filter {
                field: "a".into(),
                op: FilterOp::In,
                values: vec![],
                scope: FilterScope::Row,
            },
            Filter {
                field: "a".into(),
                op: FilterOp::In,
                values: vec![Value::Null],
                scope: FilterScope::Row,
            },
            Filter {
                field: "a".into(),
                op: FilterOp::IsNull,
                values: vec![json("x")],
                scope: FilterScope::Row,
            },
            Filter {
                field: "a".into(),
                op: FilterOp::Gt,
                values: vec![json("x"), json("y")],
                scope: FilterScope::Row,
            },
            Filter {
                field: "a".into(),
                op: FilterOp::Eq,
                values: vec![serde_json::json!([1, 2])],
                scope: FilterScope::Row,
            },
            Filter {
                field: "".into(),
                op: FilterOp::Eq,
                values: vec![json("x")],
                scope: FilterScope::Row,
            },
        ];
        for f in cases {
            let mut v = base(ChartType::Table);
            v.filters = vec![f.clone()];
            assert!(
                matches!(compile_visual(&v), Err(SqlError::InvalidSpec(_))),
                "expected rejection for {f:?}"
            );
        }
    }

    #[test]
    fn rejects_duplicate_outputs() {
        let mut v = base(ChartType::Bar);
        v.dimensions = vec!["region".into()];
        v.measures = vec![sum("revenue", Some("region"))];
        assert!(matches!(compile_visual(&v), Err(SqlError::InvalidSpec(_))));
    }

    #[test]
    fn literals_render_by_type() {
        assert_eq!(literal(&Value::Null).unwrap(), "NULL");
        assert_eq!(literal(&serde_json::json!(-7)).unwrap(), "-7");
        assert_eq!(literal(&serde_json::json!(0.25)).unwrap(), "0.25");
        assert!(literal(&serde_json::json!({"a": 1})).is_err());
    }

    fn filter(field: &str, op: FilterOp, values: Vec<Value>, scope: FilterScope) -> Filter {
        Filter {
            field: field.into(),
            op,
            values,
            scope,
        }
    }

    /// Grouped by region, with total revenue and order count measures.
    fn grouped_visual() -> Visual {
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into()];
        v.measures = vec![
            sum("revenue", Some("total")),
            Measure {
                field: "order_id".into(),
                agg: Aggregate::CountDistinct,
                alias: Some("orders".into()),
            },
        ];
        v
    }

    #[test]
    fn aggregate_filters_compile_to_having_over_the_expression() {
        let mut v = grouped_visual();
        v.filters = vec![filter(
            "total",
            FilterOp::Gt,
            vec![serde_json::json!(100)],
            FilterScope::Aggregate,
        )];
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "region", SUM("revenue") AS "total", COUNT(DISTINCT "order_id") AS "orders" FROM "sales" GROUP BY "region" HAVING SUM("revenue") > 100 ORDER BY "region" ASC"#
        );
    }

    #[test]
    fn row_and_aggregate_filters_go_to_where_and_having() {
        let mut v = grouped_visual();
        v.filters = vec![
            filter("region", FilterOp::Ne, vec![json("East")], FilterScope::Row),
            filter(
                "orders",
                FilterOp::Le,
                vec![serde_json::json!(3)],
                FilterScope::Aggregate,
            ),
            filter("total", FilterOp::IsNotNull, vec![], FilterScope::Aggregate),
        ];
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT "region", SUM("revenue") AS "total", COUNT(DISTINCT "order_id") AS "orders" FROM "sales" WHERE "region" <> 'East' GROUP BY "region" HAVING COUNT(DISTINCT "order_id") <= 3 AND SUM("revenue") IS NOT NULL ORDER BY "region" ASC"#
        );
    }

    #[test]
    fn between_renders_inclusive_range_in_both_scopes() {
        let mut v = grouped_visual();
        v.filters = vec![
            filter(
                "units",
                FilterOp::Between,
                vec![serde_json::json!(2), serde_json::json!(5)],
                FilterScope::Row,
            ),
            filter(
                "total",
                FilterOp::Between,
                vec![serde_json::json!(10.5), serde_json::json!(50)],
                FilterScope::Aggregate,
            ),
        ];
        let sql = compile_visual(&v).unwrap();
        assert!(
            sql.contains(r#"WHERE "units" BETWEEN 2 AND 5 GROUP BY"#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"HAVING SUM("revenue") BETWEEN 10.5 AND 50 ORDER"#),
            "{sql}"
        );
    }

    #[test]
    fn between_needs_exactly_two_non_null_values() {
        for values in [
            vec![serde_json::json!(1)],
            vec![
                serde_json::json!(1),
                serde_json::json!(2),
                serde_json::json!(3),
            ],
            vec![serde_json::json!(1), Value::Null],
            vec![],
        ] {
            let mut v = base(ChartType::Table);
            v.filters = vec![filter(
                "units",
                FilterOp::Between,
                values.clone(),
                FilterScope::Row,
            )];
            assert!(
                matches!(compile_visual(&v), Err(SqlError::InvalidSpec(_))),
                "{values:?}"
            );
        }
    }

    #[test]
    fn aggregate_filters_need_a_measure_output() {
        // No measures at all.
        let mut v = base(ChartType::Table);
        v.dimensions = vec!["region".into()];
        v.filters = vec![filter(
            "total",
            FilterOp::Gt,
            vec![serde_json::json!(1)],
            FilterScope::Aggregate,
        )];
        let err = compile_visual(&v).unwrap_err().to_string();
        assert!(err.contains("needs at least one measure"), "{err}");

        // A dimension or unknown name is not a measure output.
        for field in ["region", "missing"] {
            let mut v = grouped_visual();
            v.filters = vec![filter(
                field,
                FilterOp::Gt,
                vec![serde_json::json!(1)],
                FilterScope::Aggregate,
            )];
            let err = compile_visual(&v).unwrap_err().to_string();
            assert!(err.contains("is not a measure output"), "{err}");
        }
    }

    #[test]
    fn row_filters_may_not_name_a_measure_alias_unless_it_is_a_column() {
        let mut v = grouped_visual();
        v.filters = vec![filter(
            "total",
            FilterOp::Gt,
            vec![serde_json::json!(1)],
            FilterScope::Row,
        )];
        // Without the dataset's columns, an alias is refused.
        let err = compile_visual(&v).unwrap_err().to_string();
        assert!(err.contains("names a measure output"), "{err}");

        // With the columns: the alias is not one, so it is refused too.
        let columns = vec!["region".to_string(), "revenue".to_string()];
        let err = compile_visual_with(&v, Some(&columns))
            .unwrap_err()
            .to_string();
        assert!(err.contains("is not a column of the dataset"), "{err}");

        // A real column that shares the alias's name is allowed.
        let columns = vec!["region".to_string(), "total".to_string()];
        let sql = compile_visual_with(&v, Some(&columns)).unwrap();
        assert!(sql.contains(r#"WHERE "total" > 1"#), "{sql}");

        // A row filter on an unknown column is refused when columns are known.
        let mut v = base(ChartType::Table);
        v.filters = vec![filter(
            "nope",
            FilterOp::Eq,
            vec![json("x")],
            FilterScope::Row,
        )];
        assert!(compile_visual_with(&v, Some(&columns)).is_err());
    }

    #[test]
    fn aggregate_filter_values_are_escaped_like_row_filters() {
        let mut v = grouped_visual();
        v.filters = vec![filter(
            "total",
            FilterOp::Eq,
            vec![json("1); DROP TABLE x; --")],
            FilterScope::Aggregate,
        )];
        let sql = compile_visual(&v).unwrap();
        assert!(
            sql.contains(r#"HAVING SUM("revenue") = '1); DROP TABLE x; --' ORDER"#),
            "{sql}"
        );
        // A quote in a value is doubled, so it cannot end the literal.
        let mut v = grouped_visual();
        v.filters = vec![filter(
            "total",
            FilterOp::In,
            vec![json("a'b"), json("c")],
            FilterScope::Aggregate,
        )];
        let sql = compile_visual(&v).unwrap();
        assert!(sql.contains("IN ('a''b', 'c')"), "{sql}");
    }

    #[test]
    fn unbound_parameters_are_errors_not_values() {
        let mut v = base(ChartType::Table);
        v.filters = vec![filter(
            "region",
            FilterOp::Eq,
            vec![serde_json::json!({"param": "region"})],
            FilterScope::Row,
        )];
        let err = compile_visual(&v).unwrap_err().to_string();
        assert!(err.contains("parameter 'region'"), "{err}");

        let mut v = base(ChartType::Table);
        v.limit = Some(Limit::Param(dv_core::ParamRef {
            param: "top".into(),
        }));
        let err = compile_visual(&v).unwrap_err().to_string();
        assert!(err.contains("limit parameter 'top'"), "{err}");
    }

    #[test]
    fn limit_renders_bound_rows() {
        let mut v = base(ChartType::Table);
        v.limit = Some(Limit::Rows(7));
        assert_eq!(
            compile_visual(&v).unwrap(),
            r#"SELECT * FROM "sales" LIMIT 7"#
        );
    }

    fn json(s: &str) -> Value {
        Value::String(s.to_string())
    }
}
