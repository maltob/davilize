use serde_json::json;

use super::*;
use crate::model::{
    Aggregate, ChartType, Encodings, Filter, FilterOp, FilterScope, Limit, Measure, ParamRef,
    Report,
};

fn param(name: &str, kind: ParamType) -> Parameter {
    let mut p = Parameter::new(name, None);
    p.kind = Some(kind);
    p
}

fn supplied(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect()
}

#[test]
fn parses_text_by_declared_type() {
    assert_eq!(
        ParamType::String.parse_text("North").unwrap(),
        json!("North")
    );
    assert_eq!(ParamType::Number.parse_text("2.5").unwrap(), json!(2.5));
    assert_eq!(ParamType::Integer.parse_text("-7").unwrap(), json!(-7));
    assert_eq!(ParamType::Boolean.parse_text("true").unwrap(), json!(true));
    assert_eq!(
        ParamType::Date.parse_text("2024-02-29").unwrap(),
        json!("2024-02-29")
    );
}

#[test]
fn rejects_bad_text_per_type() {
    let bad: Vec<(ParamType, &str)> = vec![
        (ParamType::Number, "abc"),
        (ParamType::Number, "NaN"),
        (ParamType::Number, "inf"),
        (ParamType::Integer, "2.5"),
        (ParamType::Integer, "1e3"),
        (ParamType::Integer, ""),
        (ParamType::Boolean, "yes"),
        (ParamType::Boolean, "TRUE"),
        (ParamType::Date, "2024-13-01"),
        (ParamType::Date, "2023-02-29"),
        (ParamType::Date, "2024-1-01"),
        (ParamType::Date, "2024/01/01"),
        (ParamType::Date, "20240101"),
        (ParamType::Date, "２024-01-01"),
        (ParamType::String, "a\0b"),
    ];
    for (kind, text) in bad {
        assert!(
            kind.parse_text(text).is_err(),
            "{kind:?} should reject {text:?}"
        );
    }
}

#[test]
fn leap_years_follow_the_gregorian_rule() {
    assert!(parse_date("2024-02-29").is_ok());
    assert!(parse_date("2000-02-29").is_ok());
    assert!(parse_date("1900-02-29").is_err());
    assert!(parse_date("2024-04-31").is_err());
    assert!(parse_date("2024-12-31").is_ok());
}

#[test]
fn json_values_are_checked_against_the_declared_type() {
    assert!(ParamType::Integer.coerce_json(&json!(3)).is_ok());
    assert!(ParamType::Integer.coerce_json(&json!(3.5)).is_err());
    assert!(ParamType::Number.coerce_json(&json!(3)).is_ok());
    assert!(ParamType::Number.coerce_json(&json!("3")).is_err());
    assert!(ParamType::Boolean.coerce_json(&json!(1)).is_err());
    assert!(ParamType::Date.coerce_json(&json!("2024-06-30")).is_ok());
    assert!(ParamType::Date.coerce_json(&json!("2024-06-31")).is_err());
}

#[test]
fn declared_type_wins_and_defaults_to_the_default_value_type() {
    assert_eq!(Parameter::new("a", None).param_type(), ParamType::String);
    assert_eq!(
        Parameter::new("a", Some(json!(2024))).param_type(),
        ParamType::Integer
    );
    assert_eq!(
        Parameter::new("a", Some(json!(2.5))).param_type(),
        ParamType::Number
    );
    assert_eq!(
        Parameter::new("a", Some(json!(false))).param_type(),
        ParamType::Boolean
    );
    let mut p = Parameter::new("a", Some(json!(2024)));
    p.kind = Some(ParamType::Date);
    assert_eq!(p.param_type(), ParamType::Date);
}

#[test]
fn declarations_are_validated() {
    // Default of the wrong type.
    let mut p = param("n", ParamType::Integer);
    p.default = Some(json!("ten"));
    assert!(validate_parameter(&p).is_err());

    // Default outside the allowed values or the bounds.
    let mut p = param("region", ParamType::String);
    p.default = Some(json!("Mars"));
    p.allowed = Some(vec![json!("North"), json!("South")]);
    assert!(validate_parameter(&p).is_err());

    let mut p = param("n", ParamType::Integer);
    p.default = Some(json!(11));
    p.max = Some(json!(10));
    assert!(validate_parameter(&p).is_err());

    // min above max, and bounds on a type that has no order.
    let mut p = param("n", ParamType::Number);
    p.min = Some(json!(5));
    p.max = Some(json!(1));
    assert!(validate_parameter(&p).is_err());
    let mut p = param("flag", ParamType::Boolean);
    p.min = Some(json!(0));
    assert!(validate_parameter(&p).is_err());

    // Empty allowed list.
    let mut p = param("s", ParamType::String);
    p.allowed = Some(vec![]);
    assert!(validate_parameter(&p).is_err());

    // A consistent declaration passes.
    let mut p = param("year", ParamType::Integer);
    p.default = Some(json!(2024));
    p.min = Some(json!(2000));
    p.max = Some(json!(2030));
    assert!(validate_parameter(&p).is_ok());
}

#[test]
fn resolve_uses_supplied_values_then_defaults() {
    let mut year = param("year", ParamType::Integer);
    year.default = Some(json!(2024));
    let mut region = param("region", ParamType::String);
    region.allowed = Some(vec![json!("North"), json!("South")]);
    region.default = Some(json!("North"));
    let day = {
        let mut p = param("day", ParamType::Date);
        p.min = Some(json!("2024-01-01"));
        p.max = Some(json!("2024-12-31"));
        p
    };
    let params = vec![year, region, day];

    let values = resolve_parameters(
        &params,
        &supplied(&[("year", "2023"), ("day", "2024-03-01")]),
    )
    .unwrap();
    assert_eq!(values["year"], json!(2023));
    assert_eq!(values["region"], json!("North"));
    assert_eq!(values["day"], json!("2024-03-01"));

    let values = resolve_parameters(&params, &supplied(&[("day", "2024-03-01")])).unwrap();
    assert_eq!(
        values["year"],
        json!(2024),
        "default is used when not supplied"
    );
}

#[test]
fn resolve_reports_unknown_bad_missing_and_repeated_parameters() {
    let mut year = param("year", ParamType::Integer);
    year.default = Some(json!(2024));
    let required = param("region", ParamType::String); // no default
    let params = vec![year, required];

    let unknown = resolve_parameters(&params, &supplied(&[("region", "x"), ("nope", "1")]));
    assert!(
        matches!(unknown, Err(CoreError::Invalid(m)) if m.contains("unknown parameter 'nope'"))
    );

    let bad = resolve_parameters(&params, &supplied(&[("region", "x"), ("year", "soon")]));
    assert!(matches!(bad, Err(CoreError::Invalid(m)) if m.contains("'soon' is not an integer")));

    let missing = resolve_parameters(&params, &supplied(&[("year", "2020")]));
    assert!(
        matches!(missing, Err(CoreError::Invalid(m)) if m.contains("missing required parameter 'region'"))
    );

    let repeated = resolve_parameters(&params, &supplied(&[("region", "a"), ("region", "b")]));
    assert!(matches!(repeated, Err(CoreError::Invalid(m)) if m.contains("more than once")));
}

#[test]
fn resolve_enforces_allowed_values_and_bounds_on_supplied_text() {
    let mut year = param("year", ParamType::Integer);
    year.min = Some(json!(2000));
    year.max = Some(json!(2030));
    year.default = Some(json!(2024));
    let params = vec![year];
    assert!(resolve_parameters(&params, &supplied(&[("year", "1999")])).is_err());
    assert!(resolve_parameters(&params, &supplied(&[("year", "2031")])).is_err());
    assert!(resolve_parameters(&params, &supplied(&[("year", "2030")])).is_ok());

    let mut region = param("region", ParamType::String);
    region.allowed = Some(vec![json!("North")]);
    let params = vec![region];
    let err =
        resolve_parameters(&params, &supplied(&[("region", "North; DROP TABLE x")])).unwrap_err();
    assert!(err.to_string().contains("not one of the allowed values"));
}

#[test]
fn no_parameters_declared_and_none_supplied_is_fine() {
    assert!(resolve_parameters(&[], &[]).unwrap().is_empty());
    let err = resolve_parameters(&[], &supplied(&[("x", "1")])).unwrap_err();
    assert!(err.to_string().contains("declared: none"));
}

#[test]
fn param_refs_must_have_exactly_one_string_key() {
    assert_eq!(param_ref(&json!("North")).unwrap(), None);
    assert_eq!(param_ref(&json!({"param": "year"})).unwrap(), Some("year"));
    assert!(param_ref(&json!({"param": 3})).is_err());
    assert!(param_ref(&json!({"param": "y", "other": 1})).is_err());
}

fn visual_with_param_filter_and_limit() -> Visual {
    Visual {
        id: "v".into(),
        title: "t".into(),
        dataset: "sales".into(),
        chart: ChartType::Table,
        encodings: Encodings::default(),
        dimensions: vec!["region".into()],
        measures: vec![Measure {
            field: "revenue".into(),
            agg: Aggregate::Sum,
            alias: Some("total".into()),
        }],
        filters: vec![
            Filter {
                field: "region".into(),
                op: FilterOp::Eq,
                values: vec![json!({"param": "region"})],
                scope: FilterScope::Row,
            },
            Filter {
                field: "total".into(),
                op: FilterOp::Between,
                values: vec![json!(0), json!({"param": "cap"})],
                scope: FilterScope::Aggregate,
            },
        ],
        sort: None,
        limit: Some(Limit::Param(ParamRef {
            param: "top".into(),
        })),
    }
}

#[test]
fn bind_visual_replaces_references_with_values() {
    let mut values = ParamValues::new();
    values.insert("region".into(), json!("North"));
    values.insert("cap".into(), json!(500.5));
    values.insert("top".into(), json!(10.0));
    let bound = bind_visual(&visual_with_param_filter_and_limit(), &values).unwrap();
    assert_eq!(bound.filters[0].values, vec![json!("North")]);
    assert_eq!(bound.filters[1].values, vec![json!(0), json!(500.5)]);
    assert_eq!(bound.limit, Some(Limit::Rows(10)));
}

#[test]
fn bind_visual_rejects_unknown_names_and_bad_limits() {
    let mut values = ParamValues::new();
    values.insert("region".into(), json!("North"));
    values.insert("cap".into(), json!(1));
    values.insert("top".into(), json!(2.5));
    let err = bind_visual(&visual_with_param_filter_and_limit(), &values).unwrap_err();
    assert!(err.to_string().contains("whole number"));

    values.insert("top".into(), json!(3));
    values.remove("cap");
    let err = bind_visual(&visual_with_param_filter_and_limit(), &values).unwrap_err();
    assert!(err.to_string().contains("unknown parameter 'cap'"));
}

#[test]
fn report_validation_checks_parameter_references() {
    let base = || {
        let mut report = Report::new("r");
        report.sources.push(crate::model::Source {
            name: "s".into(),
            kind: crate::model::SourceKind::File {
                path: "s.csv".into(),
                format: crate::model::FileFormat::Csv,
            },
        });
        report.datasets.push(crate::model::Dataset {
            name: "sales".into(),
            source: "s".into(),
            sql: None,
            joins: Vec::new(),
            calculated_fields: vec![],
        });
        report
    };

    // A filter reference to an undeclared parameter is rejected.
    let mut report = base();
    report.parameters.push(Parameter::new("known", None));
    let mut visual = visual_with_param_filter_and_limit();
    visual.limit = None;
    visual.filters.truncate(1);
    visual.filters[0].values = vec![json!({"param": "missing"})];
    report.visuals.push(visual);
    let err = report.validate().unwrap_err();
    assert!(err.to_string().contains("unknown parameter 'missing'"));

    // A limit reference to an undeclared parameter is rejected.
    let mut report = base();
    report.visuals.push(visual_with_param_filter_and_limit());
    assert!(report.validate().is_err());

    // Declared references pass.
    let mut report = base();
    for name in ["region", "cap", "top"] {
        report.parameters.push(Parameter::new(name, None));
    }
    report.visuals.push(visual_with_param_filter_and_limit());
    assert!(report.validate().is_ok());
}

#[test]
fn validation_runs_for_declared_parameters_in_reports() {
    let mut report = Report::new("r");
    let mut p = param("year", ParamType::Integer);
    p.default = Some(json!("2024"));
    report.parameters.push(p);
    assert!(
        report.validate().is_err(),
        "a string default for an integer is rejected"
    );
}
