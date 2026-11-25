//! Validation of calculated-field expressions.
//!
//! `.dvz` files may come from other people, so an expression is not trusted as
//! SQL text. DuckDB parses it (`json_serialize_sql`) and the resulting syntax tree
//! is checked against an allowlist: column references, constants, operators,
//! comparisons, CASE, CAST, BETWEEN, and a fixed set of scalar functions. Anything
//! else is refused with the field's name in the message: subqueries, window and
//! aggregate functions, table functions (read_csv, glob, ...), parameters, `*`,
//! catalog-qualified names, and text tricks such as `;`, comments and `attach`.
//!
//! The same allowlist is documented in docs/reports.md. Change both together.

use serde_json::Value;

use crate::SqlError;

/// Longest accepted expression, in characters.
pub const MAX_EXPRESSION_CHARS: usize = 2000;

/// Deepest syntax tree accepted (guards the recursive walk).
const MAX_DEPTH: usize = 200;

/// Scalar functions allowed in calculated fields, by DuckDB's lowercase name.
/// Excluded on purpose: anything that reads outside the row (table functions,
/// `getenv`, `current_setting`, `read_*`), anything time-dependent or random, and
/// functions that can allocate unbounded memory from a small input (`repeat`,
/// `lpad`, `rpad`).
pub const ALLOWED_FUNCTIONS: &[&str] = &[
    // Math
    "abs",
    "round",
    "round_even",
    "floor",
    "ceil",
    "ceiling",
    "trunc",
    "sqrt",
    "cbrt",
    "power",
    "pow",
    "exp",
    "ln",
    "log",
    "log10",
    "log2",
    "mod",
    "sign",
    "pi",
    "degrees",
    "radians",
    "sin",
    "cos",
    "tan",
    "asin",
    "acos",
    "atan",
    "atan2",
    "greatest",
    "least",
    // Text
    "length",
    "char_length",
    "lower",
    "upper",
    "trim",
    "ltrim",
    "rtrim",
    "substring",
    "substr",
    "replace",
    "concat",
    "concat_ws",
    "left",
    "right",
    "reverse",
    "starts_with",
    "ends_with",
    "contains",
    // Conditional and null handling
    "coalesce",
    "ifnull",
    "nullif",
    "if",
    "iif",
    // Dates and times (no clock access: current_date and now are not allowed)
    "year",
    "month",
    "day",
    "hour",
    "minute",
    "second",
    "dayofweek",
    "dayofyear",
    "weekday",
    "week",
    "quarter",
    "date_part",
    "date_trunc",
    "datediff",
    "date_diff",
    "date_add",
    "date_sub",
    "make_date",
    "strftime",
    "strptime",
];

/// Types allowed as CAST targets (uppercase).
pub const ALLOWED_TYPES: &[&str] = &[
    "BOOLEAN",
    "TINYINT",
    "SMALLINT",
    "INTEGER",
    "BIGINT",
    "HUGEINT",
    "UTINYINT",
    "USMALLINT",
    "UINTEGER",
    "UBIGINT",
    "FLOAT",
    "REAL",
    "DOUBLE",
    "DECIMAL",
    "NUMERIC",
    "VARCHAR",
    "TEXT",
    "STRING",
    "CHAR",
    "DATE",
    "TIME",
    "TIMESTAMP",
    "INTERVAL",
];

/// Operators allowed in calculated fields (DuckDB reports them as functions with
/// `is_operator`). Pattern matching with LIKE is included.
pub const ALLOWED_OPERATORS: &[&str] = &[
    "+", "-", "*", "/", "//", "%", "^", "**", "||", "=", "<>", "!=", "<", "<=", ">", ">=", "~~",
    "!~~", "~~*", "!~~*",
];

/// Rejects text that has no place in a single-line expression, before parsing.
/// Run this before handing the expression to the SQL parser.
pub fn precheck(field: &str, expression: &str) -> Result<(), SqlError> {
    check_text(expression).map_err(|reason| fail(field, reason))
}

/// The SQL that asks DuckDB to parse `expression`; its single text result goes to [`verify`].
pub fn parse_query(expression: &str) -> String {
    let wrapped = format!("SELECT ({expression})");
    format!(
        "SELECT json_serialize_sql({})",
        crate::quote_literal(&wrapped)
    )
}

/// Checks the parse tree DuckDB returned (`json_serialize_sql`) against the allowlist.
pub fn verify(field: &str, serialized: &str) -> Result<(), SqlError> {
    let root: Value = serde_json::from_str(serialized)
        .map_err(|e| fail(field, format!("cannot read the parsed expression: {e}")))?;
    if root["error"] == Value::Bool(true) {
        let message = root["error_message"].as_str().unwrap_or("syntax error");
        return Err(fail(field, format!("invalid expression: {message}")));
    }
    check_tree(&root).map_err(|reason| fail(field, reason))
}

fn fail(field: &str, reason: String) -> SqlError {
    SqlError::InvalidSpec(format!("calculated field '{field}': {reason}"))
}

/// Rejects text that has no place in a single-line expression, before parsing.
fn check_text(expression: &str) -> Result<(), String> {
    if expression.trim().is_empty() {
        return Err("the expression is empty".to_string());
    }
    if expression.chars().count() > MAX_EXPRESSION_CHARS {
        return Err(format!(
            "the expression is longer than {MAX_EXPRESSION_CHARS} characters"
        ));
    }
    if expression
        .chars()
        .any(|c| c == '\0' || (c.is_control() && !c.is_whitespace()))
    {
        return Err("control characters are not allowed".to_string());
    }
    for (token, why) in [
        (";", "statements cannot be combined"),
        ("--", "comments are not allowed"),
        ("/*", "comments are not allowed"),
        ("*/", "comments are not allowed"),
    ] {
        if expression.contains(token) {
            return Err(format!("'{token}' is not allowed ({why})"));
        }
    }
    Ok(())
}

/// The parsed statement must be exactly one projection with no source, no
/// filters, grouping, modifiers or CTEs. Then the one expression is checked.
fn check_tree(root: &Value) -> Result<(), String> {
    let single = "the expression must be a single value expression";
    let statements = root["statements"]
        .as_array()
        .filter(|s| s.len() == 1)
        .ok_or(single)?;
    let statement = &statements[0];
    let node = &statement["node"];
    if node["type"] != "SELECT_NODE" || !empty_or_null(&statement["named_param_map"]) {
        return Err(single.to_string());
    }
    let projections = node["select_list"]
        .as_array()
        .filter(|l| l.len() == 1)
        .ok_or(single)?;
    let no_source = node["from_table"]["type"] == "EMPTY";
    let nothing_else = node["where_clause"].is_null()
        && node["having"].is_null()
        && node["qualify"].is_null()
        && node["sample"].is_null()
        && empty_or_null(&node["group_expressions"])
        && empty_or_null(&node["group_sets"])
        && empty_or_null(&node["modifiers"])
        && empty_or_null(&node["cte_map"]["map"]);
    if !(no_source && nothing_else) {
        return Err(
            "the expression cannot use FROM, WHERE, GROUP BY, HAVING, ORDER BY, LIMIT or WITH"
                .to_string(),
        );
    }
    walk(&projections[0], 0)
}

fn empty_or_null(value: &Value) -> bool {
    value.is_null() || value.as_array().is_some_and(Vec::is_empty)
}

/// Visits every node of the expression. Nodes with a `class` are checked; the
/// other objects and arrays (arguments, literals, types) are walked into.
fn walk(value: &Value, depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err("the expression is nested too deeply".to_string());
    }
    match value {
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("SELECT_NODE") {
                return Err("subqueries are not allowed".to_string());
            }
            // An alias (`expr AS name`, or a bare word after an expression) has no place here.
            if map
                .get("alias")
                .and_then(Value::as_str)
                .is_some_and(|a| !a.is_empty())
            {
                return Err("aliases are not allowed in calculated fields".to_string());
            }
            if let Some(class) = map.get("class").and_then(Value::as_str) {
                check_node(class, value)?;
            }
            for child in map.values() {
                walk(child, depth + 1)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|item| walk(item, depth + 1)),
        _ => Ok(()),
    }
}

fn check_node(class: &str, node: &Value) -> Result<(), String> {
    match class {
        "COLUMN_REF" | "CONSTANT" | "COMPARISON" | "CONJUNCTION" | "OPERATOR" | "BETWEEN"
        | "CASE" | "CAST" => Ok(()),
        "TYPE" => check_type(node),
        "FUNCTION" => check_function(node),
        "SUBQUERY" => Err("subqueries are not allowed".to_string()),
        "WINDOW" => Err("window functions are not allowed".to_string()),
        "STAR" => Err("'*' is not allowed in an expression".to_string()),
        "PARAMETER" => Err("parameters are not allowed in calculated fields".to_string()),
        other => Err(format!(
            "'{}' is not allowed in calculated fields",
            other.to_ascii_lowercase()
        )),
    }
}

/// CAST targets. Only built-in scalar types: `'text' AS attach` and similar
/// spellings then fail here instead of being read as something else.
fn check_type(node: &Value) -> Result<(), String> {
    let name = node["type_name"].as_str().unwrap_or("");
    let qualified = |key: &str| node[key].as_str().is_some_and(|s| !s.is_empty());
    if qualified("catalog") || qualified("schema") {
        return Err(format!("qualified type name '{name}' is not allowed"));
    }
    if ALLOWED_TYPES.contains(&name.to_ascii_uppercase().as_str()) {
        Ok(())
    } else {
        Err(format!(
            "type '{name}' is not allowed in a cast (use a built-in scalar type)"
        ))
    }
}

fn check_function(node: &Value) -> Result<(), String> {
    let name = node["function_name"].as_str().unwrap_or("");
    let is_operator = node["is_operator"].as_bool().unwrap_or(false);
    if node["distinct"] == Value::Bool(true) {
        return Err(format!("DISTINCT in '{name}' is not allowed"));
    }
    if !node["filter"].is_null() {
        return Err(format!("FILTER in '{name}' is not allowed"));
    }
    if !empty_or_null(&node["order_bys"]["orders"]) {
        return Err(format!("ORDER BY in '{name}' is not allowed"));
    }
    if node["export_state"] == Value::Bool(true) {
        return Err(format!("'{name}' is not allowed"));
    }
    let qualified = |key: &str| node[key].as_str().is_some_and(|s| !s.is_empty());
    if qualified("catalog") || qualified("schema") {
        return Err(format!("qualified function name '{name}' is not allowed"));
    }
    if is_operator {
        if ALLOWED_OPERATORS.contains(&name) {
            return Ok(());
        }
        return Err(format!("operator '{name}' is not allowed"));
    }
    let lower = name.to_ascii_lowercase();
    if ALLOWED_FUNCTIONS.contains(&lower.as_str()) {
        Ok(())
    } else {
        Err(format!(
            "function '{name}' is not allowed (see docs/reports.md for the allowed functions)"
        ))
    }
}
