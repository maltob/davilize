//! Report parameters: declared types and constraints, validation of supplied
//! text, and binding of `{"param": ...}` references in a visual.
//!
//! Everything here is pure (no DuckDB). Supplied text is parsed according to the
//! declared type; the result is a JSON scalar that the engine renders with its
//! usual typed-literal function, so parameter text never reaches the SQL as raw
//! text.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde_json::{Number, Value};

use crate::CoreError;
use crate::model::{Filter, Limit, ParamType, Parameter, Visual};

/// Bound parameter values by name, after type checks and defaults are applied.
pub type ParamValues = BTreeMap<String, Value>;

impl ParamType {
    /// The name used in reports and in `davilize param list`.
    pub fn as_str(self) -> &'static str {
        match self {
            ParamType::String => "string",
            ParamType::Number => "number",
            ParamType::Integer => "integer",
            ParamType::Boolean => "boolean",
            ParamType::Date => "date",
        }
    }

    /// Parses text supplied on the command line.
    pub fn parse_text(self, text: &str) -> Result<Value, String> {
        match self {
            ParamType::String => check_string(text).map(|_| Value::String(text.to_string())),
            ParamType::Number => {
                let f: f64 = text
                    .parse()
                    .map_err(|_| format!("'{text}' is not a number"))?;
                Number::from_f64(f)
                    .map(Value::Number)
                    .ok_or_else(|| format!("'{text}' is not a finite number"))
            }
            ParamType::Integer => text
                .parse::<i64>()
                .map(Value::from)
                .map_err(|_| format!("'{text}' is not an integer")),
            ParamType::Boolean => match text {
                "true" => Ok(Value::Bool(true)),
                "false" => Ok(Value::Bool(false)),
                _ => Err(format!("'{text}' is not a boolean (use true or false)")),
            },
            ParamType::Date => parse_date(text)
                .map(|_| Value::String(text.to_string()))
                .map_err(|e| e.to_string()),
        }
    }

    /// Checks a JSON value written in the report (a default, an allowed value or a bound).
    pub fn coerce_json(self, value: &Value) -> Result<Value, String> {
        let ok = match (self, value) {
            (ParamType::String, Value::String(s)) => check_string(s).is_ok(),
            (ParamType::Number, Value::Number(n)) => n.as_f64().is_some_and(f64::is_finite),
            (ParamType::Integer, Value::Number(n)) => n.is_i64(),
            (ParamType::Boolean, Value::Bool(_)) => true,
            (ParamType::Date, Value::String(s)) => parse_date(s).is_ok(),
            _ => false,
        };
        if ok {
            Ok(value.clone())
        } else {
            Err(format!("expected a {} value, got {value}", self.as_str()))
        }
    }
}

fn check_string(text: &str) -> Result<(), String> {
    if text.contains('\0') {
        Err("text must not contain NUL characters".to_string())
    } else {
        Ok(())
    }
}

/// Parses `YYYY-MM-DD` (strictly: four, two and two ASCII digits) into year, month, day.
pub fn parse_date(text: &str) -> Result<(u32, u32, u32), String> {
    let bytes = text.as_bytes();
    let shape_ok = bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !shape_ok {
        return Err(format!("'{text}' is not a date (expected YYYY-MM-DD)"));
    }
    let year: u32 = text[0..4]
        .parse()
        .map_err(|_| format!("'{text}' is not a date"))?;
    let month: u32 = text[5..7]
        .parse()
        .map_err(|_| format!("'{text}' is not a date"))?;
    let day: u32 = text[8..10]
        .parse()
        .map_err(|_| format!("'{text}' is not a date"))?;
    if !(1..=12).contains(&month) {
        return Err(format!("'{text}' has a month outside 01-12"));
    }
    if day < 1 || day > days_in_month(year, month) {
        return Err(format!(
            "'{text}' has a day that does not exist in that month"
        ));
    }
    Ok((year, month, day))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        _ => 31,
    }
}

impl Parameter {
    /// The declared type, or the type implied by `default` (string when there is none).
    pub fn param_type(&self) -> ParamType {
        if let Some(kind) = self.kind {
            return kind;
        }
        match &self.default {
            Some(Value::Bool(_)) => ParamType::Boolean,
            Some(Value::Number(n)) if n.is_i64() => ParamType::Integer,
            Some(Value::Number(_)) => ParamType::Number,
            _ => ParamType::String,
        }
    }
}

/// Checks a parameter's declaration: its default, allowed values and bounds must
/// match its type, and `min` must not exceed `max`.
pub fn validate_parameter(parameter: &Parameter) -> Result<(), String> {
    let kind = parameter.param_type();
    let name = &parameter.name;
    let wrap = |e: String| format!("parameter '{name}': {e}");

    if (parameter.min.is_some() || parameter.max.is_some())
        && !matches!(
            kind,
            ParamType::Number | ParamType::Integer | ParamType::Date
        )
    {
        return Err(wrap(format!(
            "min and max apply to number, integer and date parameters, not {}",
            kind.as_str()
        )));
    }
    let min = parameter
        .min
        .as_ref()
        .map(|v| kind.coerce_json(v).map_err(wrap))
        .transpose()?;
    let max = parameter
        .max
        .as_ref()
        .map(|v| kind.coerce_json(v).map_err(wrap))
        .transpose()?;
    if let (Some(lo), Some(hi)) = (&min, &max)
        && compare(lo, hi) == Some(Ordering::Greater)
    {
        return Err(wrap("min is greater than max".to_string()));
    }
    if let Some(allowed) = &parameter.allowed {
        if allowed.is_empty() {
            return Err(wrap("the allowed list is empty".to_string()));
        }
        for value in allowed {
            let value = kind.coerce_json(value).map_err(wrap)?;
            check_constraints(parameter, &value).map_err(wrap)?;
        }
    }
    if let Some(default) = &parameter.default {
        let value = kind.coerce_json(default).map_err(wrap)?;
        check_constraints(parameter, &value).map_err(wrap)?;
    }
    Ok(())
}

/// Checks an already type-correct value against `allowed`, `min` and `max`.
fn check_constraints(parameter: &Parameter, value: &Value) -> Result<(), String> {
    if let Some(allowed) = &parameter.allowed
        && !allowed.iter().any(|a| equivalent(a, value))
    {
        return Err(format!("{value} is not one of the allowed values"));
    }
    if let Some(min) = &parameter.min
        && compare(value, min) == Some(Ordering::Less)
    {
        return Err(format!("{value} is below the minimum {min}"));
    }
    if let Some(max) = &parameter.max
        && compare(value, max) == Some(Ordering::Greater)
    {
        return Err(format!("{value} is above the maximum {max}"));
    }
    Ok(())
}

/// Orders two type-correct values: numbers numerically, strings (including ISO dates)
/// lexicographically. `None` when the values are not comparable.
fn compare(a: &Value, b: &Value) -> Option<Ordering> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64()?.partial_cmp(&y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x.cmp(y)),
        _ => None,
    }
}

/// Equality that treats `2024` and `2024.0` as the same number.
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(_), Value::Number(_)) => compare(a, b) == Some(Ordering::Equal),
        _ => a == b,
    }
}

/// Reads a `{"param": "<name>"}` reference. `Ok(None)` for any other value.
/// An object with a `param` key that is not exactly that shape is an error.
pub fn param_ref(value: &Value) -> Result<Option<&str>, CoreError> {
    let Value::Object(map) = value else {
        return Ok(None);
    };
    if !map.contains_key("param") {
        return Ok(None);
    }
    match (map.len(), map.get("param")) {
        (1, Some(Value::String(name))) => Ok(Some(name.as_str())),
        _ => Err(CoreError::Invalid(
            "a parameter reference must look like {\"param\": \"name\"}".to_string(),
        )),
    }
}

/// Resolves supplied `(name, text)` pairs against the declared parameters:
/// unknown and repeated names are errors, missing values fall back to the default,
/// and a parameter without a default must be supplied.
pub fn resolve_parameters(
    parameters: &[Parameter],
    supplied: &[(String, String)],
) -> Result<ParamValues, CoreError> {
    let declared: Vec<&str> = parameters.iter().map(|p| p.name.as_str()).collect();
    let mut seen = Vec::new();
    for (name, _) in supplied {
        if !declared.contains(&name.as_str()) {
            return Err(CoreError::Invalid(format!(
                "unknown parameter '{name}' (declared: {})",
                if declared.is_empty() {
                    "none".to_string()
                } else {
                    declared.join(", ")
                }
            )));
        }
        if seen.contains(&name) {
            return Err(CoreError::Invalid(format!(
                "parameter '{name}' was given more than once"
            )));
        }
        seen.push(name);
    }

    let mut values = ParamValues::new();
    for parameter in parameters {
        let kind = parameter.param_type();
        let supplied_text = supplied
            .iter()
            .find(|(name, _)| name == &parameter.name)
            .map(|(_, text)| text);
        let value = match (supplied_text, &parameter.default) {
            (Some(text), _) => {
                let value = kind.parse_text(text).map_err(|e| {
                    CoreError::Invalid(format!("parameter '{}': {e}", parameter.name))
                })?;
                check_constraints(parameter, &value).map_err(|e| {
                    CoreError::Invalid(format!("parameter '{}': {e}", parameter.name))
                })?;
                value
            }
            (None, Some(default)) => kind
                .coerce_json(default)
                .map_err(|e| CoreError::Invalid(format!("parameter '{}': {e}", parameter.name)))?,
            (None, None) => {
                return Err(CoreError::Invalid(format!(
                    "missing required parameter '{}' (it has no default)",
                    parameter.name
                )));
            }
        };
        values.insert(parameter.name.clone(), value);
    }
    Ok(values)
}

/// Returns a copy of `visual` with every `{"param": ...}` filter value and a
/// parameter `limit` replaced by its bound value.
pub fn bind_visual(visual: &Visual, values: &ParamValues) -> Result<Visual, CoreError> {
    let lookup = |name: &str| {
        values
            .get(name)
            .cloned()
            .ok_or_else(|| CoreError::Invalid(format!("unknown parameter '{name}'")))
    };
    let mut bound = visual.clone();
    for filter in &mut bound.filters {
        bind_filter(filter, &lookup)?;
    }
    if let Some(Limit::Param(reference)) = &visual.limit {
        let value = lookup(&reference.param)?;
        let rows = value
            .as_f64()
            .filter(|f| f.fract() == 0.0 && *f >= 0.0 && *f <= u64::MAX as f64)
            .ok_or_else(|| {
                CoreError::Invalid(format!(
                    "parameter '{}' is used as a limit, so it must be a whole number of rows",
                    reference.param
                ))
            })?;
        bound.limit = Some(Limit::Rows(rows as u64));
    }
    Ok(bound)
}

fn bind_filter(
    filter: &mut Filter,
    lookup: &impl Fn(&str) -> Result<Value, CoreError>,
) -> Result<(), CoreError> {
    for value in &mut filter.values {
        let name = param_ref(value)?.map(str::to_owned);
        if let Some(name) = name {
            *value = lookup(&name)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
