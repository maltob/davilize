use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::CoreError;

/// Current `.dvz` schema version. Files with any other version are rejected.
pub const REPORT_VERSION: u32 = 1;

/// A `.dvz` report file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default)]
    pub datasets: Vec<Dataset>,
    #[serde(default)]
    pub visuals: Vec<Visual>,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
}

impl Report {
    /// Creates an empty report with the current schema version.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: REPORT_VERSION,
            name: name.into(),
            sources: Vec::new(),
            datasets: Vec::new(),
            visuals: Vec::new(),
            parameters: Vec::new(),
        }
    }

    /// Parses and validates a report from JSON text.
    pub fn from_json(text: &str) -> Result<Self, CoreError> {
        let report: Report = serde_json::from_str(text)?;
        report.validate()?;
        Ok(report)
    }

    /// Serializes the report as pretty-printed JSON with a trailing newline.
    pub fn to_json(&self) -> Result<String, CoreError> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// Loads and validates a report from a `.dvz` file.
    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let text = std::fs::read_to_string(path).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_json(&text)
    }

    /// Validates and writes the report to `path`.
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        self.validate()?;
        let text = self.to_json()?;
        std::fs::write(path, text).map_err(|source| CoreError::Io {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Checks version, name uniqueness and cross-references between sections.
    pub fn validate(&self) -> Result<(), CoreError> {
        if self.version != REPORT_VERSION {
            return Err(CoreError::UnsupportedVersion {
                found: self.version,
                supported: REPORT_VERSION,
            });
        }

        // Sources and datasets are both registered as relations, so they share a namespace.
        let mut relation_names = HashSet::new();
        for source in &self.sources {
            check_name("source", &source.name)?;
            if !relation_names.insert(source.name.as_str()) {
                return Err(invalid(format!("duplicate name '{}'", source.name)));
            }
        }
        for dataset in &self.datasets {
            check_name("dataset", &dataset.name)?;
            if !relation_names.insert(dataset.name.as_str()) {
                return Err(invalid(format!(
                    "dataset '{}' clashes with another source or dataset",
                    dataset.name
                )));
            }
            if !self.sources.iter().any(|s| s.name == dataset.source) {
                return Err(invalid(format!(
                    "dataset '{}' references unknown source '{}'",
                    dataset.name, dataset.source
                )));
            }
        }

        let dataset_names: HashSet<&str> = self.datasets.iter().map(|d| d.name.as_str()).collect();
        let mut visual_ids = HashSet::new();
        for visual in &self.visuals {
            check_name("visual id", &visual.id)?;
            if !visual_ids.insert(visual.id.as_str()) {
                return Err(invalid(format!("duplicate visual id '{}'", visual.id)));
            }
            if !dataset_names.contains(visual.dataset.as_str()) {
                return Err(invalid(format!(
                    "visual '{}' references unknown dataset '{}'",
                    visual.id, visual.dataset
                )));
            }
        }

        let mut parameter_names = HashSet::new();
        for parameter in &self.parameters {
            check_name("parameter", &parameter.name)?;
            if !parameter_names.insert(parameter.name.as_str()) {
                return Err(invalid(format!("duplicate parameter '{}'", parameter.name)));
            }
            crate::params::validate_parameter(parameter).map_err(invalid)?;
        }

        for dataset in &self.datasets {
            dataset.validate_calculated_fields()?;
        }

        // Every {"param": ...} reference must name a declared parameter.
        for visual in &self.visuals {
            for filter in &visual.filters {
                for value in &filter.values {
                    if let Some(name) = crate::params::param_ref(value)? {
                        check_declared(&parameter_names, name)?;
                    }
                }
            }
            if let Some(Limit::Param(reference)) = &visual.limit {
                check_declared(&parameter_names, &reference.param)?;
            }
        }
        Ok(())
    }
}

fn check_declared(declared: &HashSet<&str>, name: &str) -> Result<(), CoreError> {
    if declared.contains(name) {
        Ok(())
    } else {
        Err(invalid(format!("reference to unknown parameter '{name}'")))
    }
}

fn invalid(message: String) -> CoreError {
    CoreError::Invalid(message)
}

/// Names must be non-empty and made of letters, digits, `_` or `-`.
fn check_name(kind: &str, name: &str) -> Result<(), CoreError> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "{kind} name '{name}' must be non-empty and use only letters, digits, '_' or '-'"
        )))
    }
}

/// A named data source. Connection details never live in the report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    #[serde(flatten)]
    pub kind: SourceKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceKind {
    /// A file on disk. Relative paths are resolved against the report's directory.
    File { path: PathBuf, format: FileFormat },
    /// A database reached through a connection reference name (resolved outside the report).
    Database { kind: DbKind, connection: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileFormat {
    Csv,
    Parquet,
    Arrow,
    Json,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbKind {
    Sqlite,
    Postgres,
}

/// A named relation. Without `sql` it exposes the whole source; with `sql` it
/// is the result of that query over the source. Calculated fields add columns
/// computed from the relation's columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dataset {
    pub name: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calculated_fields: Vec<CalculatedField>,
}

impl Dataset {
    /// Checks the names of calculated fields. Expressions are checked by the
    /// engine, which knows the dataset's columns and the allowed functions.
    pub fn validate_calculated_fields(&self) -> Result<(), CoreError> {
        let mut seen = HashSet::new();
        for field in &self.calculated_fields {
            check_name("calculated field", &field.name)?;
            if !seen.insert(field.name.to_ascii_lowercase()) {
                return Err(invalid(format!(
                    "dataset '{}' defines calculated field '{}' twice",
                    self.name, field.name
                )));
            }
            if field.expression.trim().is_empty() {
                return Err(invalid(format!(
                    "calculated field '{}' has an empty expression",
                    field.name
                )));
            }
        }
        Ok(())
    }
}

/// A column computed from an expression over the dataset's columns, e.g.
/// `"revenue" - "cost"`. The engine restricts the expression to an allowlist
/// (see docs/reports.md).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalculatedField {
    pub name: String,
    pub expression: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A chart specification bound to one dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Visual {
    pub id: String,
    pub title: String,
    pub dataset: String,
    pub chart: ChartType,
    #[serde(default)]
    pub encodings: Encodings,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub measures: Vec<Measure>,
    #[serde(default)]
    pub filters: Vec<Filter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Sort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<Limit>,
}

/// A row limit: a number, or `{"param": "<name>"}` bound at refresh time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Limit {
    Rows(u64),
    Param(ParamRef),
}

/// A reference to a report parameter, written as `{"param": "<name>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamRef {
    pub param: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartType {
    Bar,
    Line,
    Scatter,
    Table,
    Heatmap,
}

/// Maps fields to visual channels. Values are field names or measure aliases.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Encodings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub field: String,
    pub agg: Aggregate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

impl Measure {
    /// The output column name: the explicit alias, or `<agg>_<field>`.
    pub fn output_name(&self) -> String {
        match &self.alias {
            Some(alias) => alias.clone(),
            None => format!("{}_{}", self.agg.as_str(), self.field),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregate {
    Sum,
    Avg,
    Min,
    Max,
    Count,
    CountDistinct,
}

impl Aggregate {
    pub fn as_str(self) -> &'static str {
        match self {
            Aggregate::Sum => "sum",
            Aggregate::Avg => "avg",
            Aggregate::Min => "min",
            Aggregate::Max => "max",
            Aggregate::Count => "count",
            Aggregate::CountDistinct => "count_distinct",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    /// A dataset column (row scope) or a measure output name (aggregate scope).
    pub field: String,
    pub op: FilterOp,
    /// Literal values (string, number, bool or null), or `{"param": "<name>"}`
    /// references. Empty for `is_null`/`is_not_null`. `between` takes two values.
    #[serde(default)]
    pub values: Vec<serde_json::Value>,
    /// `row` (WHERE, the default) or `aggregate` (HAVING over a measure).
    #[serde(default, skip_serializing_if = "FilterScope::is_row")]
    pub scope: FilterScope,
}

/// Where a filter applies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterScope {
    /// Filters dataset rows before grouping (WHERE).
    #[default]
    Row,
    /// Filters groups after aggregation (HAVING) on a measure's output.
    Aggregate,
}

impl FilterScope {
    pub fn is_row(&self) -> bool {
        *self == FilterScope::Row
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    In,
    /// Inclusive range: exactly two values, lower then upper.
    Between,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    /// A dimension or measure output name.
    pub by: String,
    #[serde(default = "default_direction")]
    pub direction: SortDirection,
}

fn default_direction() -> SortDirection {
    SortDirection::Asc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Asc,
    Desc,
}

/// A report parameter, bound at refresh time from `--param name=value` (or the
/// default). Values are validated against `type` and the optional constraints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    /// Used when no value is supplied. Without a default the parameter is required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<serde_json::Value>,
    /// Declared type. When absent, the type of `default` is used (string if there is none).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ParamType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The only values accepted, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<serde_json::Value>>,
    /// Inclusive lower bound (number, integer and date parameters only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<serde_json::Value>,
    /// Inclusive upper bound (number, integer and date parameters only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<serde_json::Value>,
}

impl Parameter {
    /// A parameter with only a name and an optional default (no declared type).
    pub fn new(name: impl Into<String>, default: Option<serde_json::Value>) -> Self {
        Self {
            name: name.into(),
            default,
            kind: None,
            description: None,
            allowed: None,
            min: None,
            max: None,
        }
    }
}

/// The types a parameter value can have. Text is parsed and validated per type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamType {
    String,
    Number,
    Integer,
    Boolean,
    /// An ISO calendar date, `YYYY-MM-DD`.
    Date,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report() -> Report {
        Report {
            version: REPORT_VERSION,
            name: "Sales".into(),
            sources: vec![
                Source {
                    name: "sales_csv".into(),
                    kind: SourceKind::File {
                        path: PathBuf::from("sample.csv"),
                        format: FileFormat::Csv,
                    },
                },
                Source {
                    name: "warehouse".into(),
                    kind: SourceKind::Database {
                        kind: DbKind::Postgres,
                        connection: "warehouse-prod".into(),
                    },
                },
            ],
            datasets: vec![Dataset {
                name: "sales".into(),
                source: "sales_csv".into(),
                sql: Some("SELECT * FROM sales_csv WHERE units > 0".into()),
                calculated_fields: vec![CalculatedField {
                    name: "margin".into(),
                    expression: r#""revenue" - "units""#.into(),
                    description: Some("Revenue minus units".into()),
                }],
            }],
            visuals: vec![Visual {
                id: "revenue_by_region".into(),
                title: "Revenue by region".into(),
                dataset: "sales".into(),
                chart: ChartType::Bar,
                encodings: Encodings {
                    x: Some("region".into()),
                    y: Some("total_revenue".into()),
                    color: None,
                    size: None,
                },
                dimensions: vec!["region".into()],
                measures: vec![Measure {
                    field: "revenue".into(),
                    agg: Aggregate::Sum,
                    alias: Some("total_revenue".into()),
                }],
                filters: vec![Filter {
                    field: "region".into(),
                    op: FilterOp::In,
                    values: vec![serde_json::json!("North"), serde_json::json!("South")],
                    scope: FilterScope::Row,
                }],
                sort: Some(Sort {
                    by: "total_revenue".into(),
                    direction: SortDirection::Desc,
                }),
                limit: Some(Limit::Rows(10)),
            }],
            parameters: vec![Parameter::new("year", Some(serde_json::json!(2024)))],
        }
    }

    #[test]
    fn json_round_trip_is_lossless() {
        let report = sample_report();
        let text = report.to_json().unwrap();
        let parsed = Report::from_json(&text).unwrap();
        assert_eq!(parsed, report);
        // Serializing again produces identical text (deterministic output).
        assert_eq!(parsed.to_json().unwrap(), text);
    }

    #[test]
    fn file_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.dvz");
        let report = sample_report();
        report.save(&path).unwrap();
        assert_eq!(Report::load(&path).unwrap(), report);
    }

    #[test]
    fn enums_use_snake_case_on_disk() {
        let text = sample_report().to_json().unwrap();
        assert!(text.contains("\"format\": \"csv\""));
        assert!(text.contains("\"type\": \"database\""));
        assert!(text.contains("\"agg\": \"sum\""));
        assert!(text.contains("\"op\": \"in\""));
        assert!(text.contains("\"chart\": \"bar\""));
    }

    #[test]
    fn database_source_stores_no_secrets() {
        let text = sample_report().to_json().unwrap();
        assert!(text.contains("\"connection\": \"warehouse-prod\""));
        assert!(!text.contains("password"));
        assert!(!text.contains("postgres://"));
    }

    #[test]
    fn minimal_report_defaults_missing_sections() {
        let report = Report::from_json(r#"{"version": 1, "name": "empty"}"#).unwrap();
        assert!(report.sources.is_empty());
        assert!(report.visuals.is_empty());
        assert!(report.parameters.is_empty());
    }

    #[test]
    fn rejects_unknown_version() {
        let err = Report::from_json(r#"{"version": 2, "name": "x"}"#).unwrap_err();
        assert!(matches!(
            err,
            CoreError::UnsupportedVersion {
                found: 2,
                supported: 1
            }
        ));
    }

    #[test]
    fn rejects_dangling_references() {
        let mut report = sample_report();
        report.visuals[0].dataset = "missing".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));

        let mut report = sample_report();
        report.datasets[0].source = "missing".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));
    }

    #[test]
    fn rejects_bad_names_and_duplicates() {
        let mut report = sample_report();
        report.visuals[0].id = "../evil".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));

        let mut report = sample_report();
        report.datasets[0].name = "sales_csv".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));
    }

    #[test]
    fn measure_default_output_name() {
        let m = Measure {
            field: "units".into(),
            agg: Aggregate::CountDistinct,
            alias: None,
        };
        assert_eq!(m.output_name(), "count_distinct_units");
    }

    #[test]
    fn sort_direction_defaults_to_asc() {
        let sort: Sort = serde_json::from_str(r#"{"by": "region"}"#).unwrap();
        assert_eq!(sort.direction, SortDirection::Asc);
    }

    #[test]
    fn existing_example_files_serialise_to_the_same_json() {
        // The additive fields (scope, between, calculated fields, parameter types,
        // param limits) must not change how files written before them are stored.
        for name in ["sample.dvz", "sample_db.dvz"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples")
                .join(name);
            let text = std::fs::read_to_string(&path).unwrap();
            let mut original: serde_json::Value = serde_json::from_str(&text).unwrap();
            let report = Report::from_json(&text).unwrap();
            let mut written = serde_json::to_value(&report).unwrap();
            // Visuals always wrote `filters` (even empty); that predates the new fields.
            for value in [&mut original, &mut written] {
                for visual in value["visuals"].as_array_mut().into_iter().flatten() {
                    if let Some(obj) = visual.as_object_mut()
                        && obj
                            .get("filters")
                            .is_some_and(|f| f.as_array().is_some_and(Vec::is_empty))
                    {
                        obj.remove("filters");
                    }
                }
            }
            assert_eq!(written, original, "{name}");
        }
    }

    #[test]
    fn new_fields_are_omitted_when_unused() {
        let report = Report::new("r");
        let text = report.to_json().unwrap();
        assert!(!text.contains("calculated_fields"));
        assert!(!text.contains("\"scope\""));
        let filter = Filter {
            field: "region".into(),
            op: FilterOp::Eq,
            values: vec![serde_json::json!("North")],
            scope: FilterScope::Row,
        };
        let text = serde_json::to_string(&filter).unwrap();
        assert_eq!(text, r#"{"field":"region","op":"eq","values":["North"]}"#);
    }

    #[test]
    fn scope_between_limit_param_and_parameter_types_round_trip() {
        let text = r#"{
            "version": 1,
            "name": "r",
            "sources": [{"name": "s", "type": "file", "path": "s.csv", "format": "csv"}],
            "datasets": [{"name": "d", "source": "s", "calculated_fields": [
                {"name": "margin", "expression": "a - b", "description": "x"}
            ]}],
            "visuals": [{
                "id": "v", "title": "t", "dataset": "d", "chart": "table",
                "dimensions": ["region"],
                "measures": [{"field": "revenue", "agg": "sum", "alias": "total"}],
                "filters": [
                    {"field": "total", "op": "between", "values": [0, {"param": "cap"}], "scope": "aggregate"},
                    {"field": "region", "op": "eq", "values": [{"param": "region"}]}
                ],
                "limit": {"param": "top"}
            }],
            "parameters": [
                {"name": "cap", "type": "number", "default": 100, "min": 0, "max": 1000},
                {"name": "region", "type": "string", "allowed": ["North", "South"]},
                {"name": "top", "type": "integer", "default": 5},
                {"name": "day", "type": "date", "description": "A day"}
            ]
        }"#;
        let report = Report::from_json(text).unwrap();
        let visual = &report.visuals[0];
        assert_eq!(visual.filters[0].scope, FilterScope::Aggregate);
        assert_eq!(visual.filters[0].op, FilterOp::Between);
        assert_eq!(visual.filters[1].scope, FilterScope::Row);
        assert_eq!(
            visual.limit,
            Some(Limit::Param(ParamRef {
                param: "top".into()
            }))
        );
        assert_eq!(report.datasets[0].calculated_fields[0].name, "margin");
        assert_eq!(report.parameters[3].kind, Some(ParamType::Date));

        let again = Report::from_json(&report.to_json().unwrap()).unwrap();
        assert_eq!(again, report);
    }

    #[test]
    fn rejects_duplicate_or_invalid_calculated_field_names() {
        let mut report = sample_report();
        let field = report.datasets[0].calculated_fields[0].clone();
        report.datasets[0].calculated_fields.push(CalculatedField {
            name: field.name.to_ascii_uppercase(),
            ..field.clone()
        });
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));

        let mut report = sample_report();
        report.datasets[0].calculated_fields[0].name = "bad name".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));

        let mut report = sample_report();
        report.datasets[0].calculated_fields[0].expression = "   ".into();
        assert!(matches!(report.validate(), Err(CoreError::Invalid(_))));
    }

    #[test]
    fn load_reports_io_error_with_path() {
        let err = Report::load(Path::new("definitely-missing-file.dvz")).unwrap_err();
        assert!(matches!(err, CoreError::Io { .. }));
    }
}
