//! The semantic model: TMDL text, `DataModelSchema` JSON, and CSV schemas.

use std::path::Path;

use serde_json::{Value, json};

use super::{Model, ModelColumn, ModelRelationship, ModelTable, guid};

// ------------------------------------------------------------ CSV schema

pub(super) fn split_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                current.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => fields.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    fields.push(current);
    fields
}

fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    b.len() >= 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..10].iter().all(u8::is_ascii_digit)
}

/// Reads a CSV file's header and guesses each column's type from its first rows.
pub(crate) fn csv_columns(path: &Path) -> Option<Vec<ModelColumn>> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).ok()?;
    let mut lines = BufReader::new(file).lines();
    let header = lines.next()?.ok()?;
    let names = split_csv_line(header.trim_start_matches('\u{feff}'));
    let mut kinds: Vec<Option<&'static str>> = vec![None; names.len()];
    for line in lines.take(200).map_while(Result::ok) {
        for (i, value) in split_csv_line(&line).iter().enumerate().take(names.len()) {
            let v = value.trim();
            if v.is_empty() {
                continue;
            }
            let this = if v.parse::<i64>().is_ok() {
                "int64"
            } else if v.parse::<f64>().is_ok() {
                "double"
            } else if is_date(v) {
                "dateTime"
            } else if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("false") {
                "boolean"
            } else {
                "string"
            };
            kinds[i] = Some(match (kinds[i], this) {
                (None, t) => t,
                (Some(a), b) if a == b => a,
                (Some("int64"), "double") | (Some("double"), "int64") => "double",
                _ => "string",
            });
        }
    }
    Some(
        names
            .into_iter()
            .zip(kinds)
            .map(|(name, kind)| ModelColumn {
                name,
                data_type: kind.unwrap_or("string").to_string(),
                expression: None,
            })
            .collect(),
    )
}

// ------------------------------------------------------ Power Query (M)

fn m_type(data_type: &str) -> &'static str {
    match data_type {
        "int64" => "Int64.Type",
        "double" => "type number",
        "dateTime" => "type date",
        "boolean" => "type logical",
        _ => "type text",
    }
}

/// The M query lines that load a CSV file with typed columns.
pub(crate) fn csv_query(path: &str, columns: &[ModelColumn]) -> Vec<String> {
    let escaped = path.replace('"', "\"\"");
    let types: Vec<String> = columns
        .iter()
        .filter(|c| c.expression.is_none())
        .map(|c| {
            format!(
                "{{\"{}\", {}}}",
                c.name.replace('"', "\"\""),
                m_type(&c.data_type)
            )
        })
        .collect();
    vec![
        "let".to_string(),
        format!(
            "    Source = Csv.Document(File.Contents(\"{escaped}\"), [Delimiter=\",\", Encoding=65001, QuoteStyle=QuoteStyle.Csv]),"
        ),
        "    Promoted = Table.PromoteHeaders(Source, [PromoteAllScalars=true]),".to_string(),
        format!(
            "    Typed = Table.TransformColumnTypes(Promoted, {{{}}})",
            types.join(", ")
        ),
        "in".to_string(),
        "    Typed".to_string(),
    ]
}

/// Finds the path in `File.Contents("...")` of an M query.
pub(crate) fn csv_path_from_query(query: &str) -> Option<String> {
    let at = query.find("File.Contents(")?;
    let rest = &query[at + "File.Contents(".len()..];
    let rest = rest.trim_start().strip_prefix('"')?;
    let mut path = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                path.push('"');
                chars.next();
            } else {
                break;
            }
        } else {
            path.push(c);
        }
    }
    Some(path)
}

// ---------------------------------------------------------------- TMDL

fn tmdl_name(name: &str) -> String {
    let plain = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if plain {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

pub(crate) fn tmdl_table(table: &ModelTable, m_query: Option<&[String]>) -> String {
    let mut out = format!(
        "table {}\n\tlineageTag: {}\n",
        tmdl_name(&table.name),
        guid(&table.name)
    );
    for column in &table.columns {
        out.push('\n');
        match &column.expression {
            Some(expr) => out.push_str(&format!(
                "\tcolumn {} = {}\n",
                tmdl_name(&column.name),
                expr
            )),
            None => out.push_str(&format!("\tcolumn {}\n", tmdl_name(&column.name))),
        }
        out.push_str(&format!("\t\tdataType: {}\n", column.data_type));
        out.push_str(&format!(
            "\t\tlineageTag: {}\n",
            guid(&format!("{}.{}", table.name, column.name))
        ));
        out.push_str("\t\tsummarizeBy: none\n");
        if column.expression.is_none() {
            out.push_str(&format!("\t\tsourceColumn: {}\n", column.name));
        }
    }
    for (name, expr) in &table.measures {
        out.push_str(&format!("\n\tmeasure {} = {}\n", tmdl_name(name), expr));
    }
    if let Some(query) = m_query {
        out.push_str(&format!(
            "\n\tpartition {} = m\n\t\tmode: import\n\t\tsource =\n",
            tmdl_name(&table.name)
        ));
        for line in query {
            out.push_str(&format!("\t\t\t{line}\n"));
        }
    }
    out
}

pub(crate) fn tmdl_relationships(model: &Model) -> String {
    let mut out = String::new();
    for rel in &model.relationships {
        out.push_str(&format!(
            "relationship {}\n\tfromColumn: {}.{}\n\ttoColumn: {}.{}\n\n",
            guid(&format!(
                "{}.{}>{}.{}",
                rel.from_table, rel.from_column, rel.to_table, rel.to_column
            )),
            tmdl_name(&rel.from_table),
            tmdl_name(&rel.from_column),
            tmdl_name(&rel.to_table),
            tmdl_name(&rel.to_column),
        ));
    }
    out
}

pub(crate) fn tmdl_model(model: &Model) -> String {
    let mut out = String::from(
        "model Model\n\tculture: en-US\n\tdefaultPowerBIDataSourceVersion: powerBI_V3\n\tdiscourageImplicitMeasures\n\n",
    );
    for table in &model.tables {
        out.push_str(&format!("ref table {}\n", tmdl_name(&table.name)));
    }
    out
}

/// Reads one name, quoted (`'a b'`) or bare, and returns it with the rest of the text.
fn read_tmdl_name(text: &str) -> (String, &str) {
    let text = text.trim_start();
    if let Some(rest) = text.strip_prefix('\'') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\'' {
                if matches!(chars.peek(), Some((_, '\''))) {
                    name.push('\'');
                    chars.next();
                } else {
                    return (name, &rest[i + 1..]);
                }
            } else {
                name.push(c);
            }
        }
        (name, "")
    } else {
        let end = text
            .find(|c: char| c.is_whitespace() || c == '=' || c == '.' || c == ':')
            .unwrap_or(text.len());
        (text[..end].to_string(), &text[end..])
    }
}

fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| *c == '\t').count()
}

/// Lines more indented than `base`, starting at `start`, as one block of text.
fn block_after(lines: &[&str], start: usize, base: usize) -> (String, usize) {
    let mut out = Vec::new();
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() {
            i += 1;
            continue;
        }
        if indent_of(line) <= base {
            break;
        }
        out.push(line.trim());
        i += 1;
    }
    (out.join("\n"), i)
}

pub(crate) fn parse_tmdl_table(text: &str) -> Option<ModelTable> {
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.iter().position(|l| l.starts_with("table "))?;
    let (name, _) = read_tmdl_name(&lines[first]["table ".len()..]);
    let mut table = ModelTable {
        name,
        ..ModelTable::default()
    };
    let mut i = first + 1;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() || indent_of(line) != 1 {
            i += 1;
            continue;
        }
        let body = line.trim();
        if let Some(rest) = body.strip_prefix("column ") {
            let (name, tail) = read_tmdl_name(rest);
            let mut expression = tail
                .trim()
                .strip_prefix('=')
                .map(|e| e.trim().to_string())
                .filter(|e| !e.is_empty());
            i += 1;
            if expression.is_none() && tail.trim() == "=" {
                let (block, next) = block_after(&lines, i, 1);
                expression = Some(block);
                i = next;
            }
            let mut data_type = "string".to_string();
            while i < lines.len() && (lines[i].trim().is_empty() || indent_of(lines[i]) > 1) {
                if let Some(v) = lines[i].trim().strip_prefix("dataType:") {
                    data_type = v.trim().to_string();
                }
                i += 1;
            }
            table.columns.push(ModelColumn {
                name,
                data_type,
                expression,
            });
        } else if let Some(rest) = body.strip_prefix("measure ") {
            let (name, tail) = read_tmdl_name(rest);
            i += 1;
            let mut expression = tail
                .trim()
                .strip_prefix('=')
                .map(|e| e.trim().to_string())
                .unwrap_or_default();
            if expression.is_empty() {
                let (block, next) = block_after(&lines, i, 1);
                expression = block;
                i = next;
            }
            while i < lines.len() && (lines[i].trim().is_empty() || indent_of(lines[i]) > 1) {
                i += 1;
            }
            table.measures.push((name, expression));
        } else if body.starts_with("partition ") {
            i += 1;
            let mut query = String::new();
            while i < lines.len() && (lines[i].trim().is_empty() || indent_of(lines[i]) > 1) {
                if lines[i].trim().starts_with("source") {
                    let inline = lines[i].trim().trim_start_matches("source").trim();
                    let inline = inline.strip_prefix('=').unwrap_or("").trim();
                    let (block, next) = block_after(&lines, i + 1, indent_of(lines[i]));
                    query = format!("{inline}\n{block}");
                    i = next;
                    continue;
                }
                i += 1;
            }
            if table.csv_path.is_none() {
                table.csv_path = super::model::csv_path_from_query(&query);
            }
        } else {
            i += 1;
        }
    }
    Some(table)
}

fn parse_column_ref(text: &str) -> Option<(String, String)> {
    let (table, rest) = read_tmdl_name(text);
    let rest = rest.trim_start().strip_prefix('.')?;
    let (column, _) = read_tmdl_name(rest);
    Some((table, column))
}

pub(crate) fn parse_tmdl_relationships(text: &str) -> Vec<ModelRelationship> {
    let mut out = Vec::new();
    let mut from: Option<(String, String)> = None;
    let mut to: Option<(String, String)> = None;
    let mut flush = |from: &mut Option<(String, String)>, to: &mut Option<(String, String)>| {
        if let (Some(f), Some(t)) = (from.take(), to.take()) {
            out.push(ModelRelationship {
                from_table: f.0,
                from_column: f.1,
                to_table: t.0,
                to_column: t.1,
            });
        }
    };
    for line in text.lines() {
        let body = line.trim();
        if body.starts_with("relationship ") {
            flush(&mut from, &mut to);
        } else if let Some(v) = body.strip_prefix("fromColumn:") {
            from = parse_column_ref(v);
        } else if let Some(v) = body.strip_prefix("toColumn:") {
            to = parse_column_ref(v);
        }
    }
    flush(&mut from, &mut to);
    out
}

// -------------------------------------------------- DataModelSchema JSON

pub(crate) fn schema_json(model: &Model, queries: &[(String, Vec<String>)]) -> Value {
    let tables: Vec<Value> = model
        .tables
        .iter()
        .map(|t| {
            let columns: Vec<Value> = t
                .columns
                .iter()
                .map(|c| match &c.expression {
                    Some(expr) => json!({
                        "name": c.name,
                        "dataType": c.data_type,
                        "type": "calculated",
                        "isDataTypeInferred": true,
                        "expression": expr,
                        "summarizeBy": "none",
                        "lineageTag": guid(&format!("{}.{}", t.name, c.name)),
                    }),
                    None => json!({
                        "name": c.name,
                        "dataType": c.data_type,
                        "sourceColumn": c.name,
                        "summarizeBy": "none",
                        "lineageTag": guid(&format!("{}.{}", t.name, c.name)),
                    }),
                })
                .collect();
            let measures: Vec<Value> = t
                .measures
                .iter()
                .map(|(name, expr)| json!({ "name": name, "expression": expr }))
                .collect();
            let partitions: Vec<Value> = queries
                .iter()
                .filter(|(table, _)| *table == t.name)
                .map(|(_, query)| {
                    json!({
                        "name": t.name,
                        "mode": "import",
                        "source": { "type": "m", "expression": query },
                    })
                })
                .collect();
            json!({
                "name": t.name,
                "lineageTag": guid(&t.name),
                "columns": columns,
                "measures": measures,
                "partitions": partitions,
            })
        })
        .collect();
    let relationships: Vec<Value> = model
        .relationships
        .iter()
        .map(|r| {
            json!({
                "name": guid(&format!("{}.{}>{}.{}", r.from_table, r.from_column, r.to_table, r.to_column)),
                "fromTable": r.from_table,
                "fromColumn": r.from_column,
                "toTable": r.to_table,
                "toColumn": r.to_column,
            })
        })
        .collect();
    json!({
        "name": "SemanticModel",
        "compatibilityLevel": 1550,
        "model": {
            "culture": "en-US",
            "dataAccessOptions": { "legacyRedirects": true, "returnErrorValuesAsNull": true },
            "defaultPowerBIDataSourceVersion": "powerBI_V3",
            "sourceQueryCulture": "en-US",
            "tables": tables,
            "relationships": relationships,
        },
    })
}

fn expression_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(lines) => lines
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub(crate) fn parse_schema_json(root: &Value) -> Option<Model> {
    let model = root.get("model")?;
    let mut out = Model::default();
    for t in model.get("tables")?.as_array()? {
        let name = t.get("name")?.as_str()?.to_string();
        let mut table = ModelTable {
            name,
            ..ModelTable::default()
        };
        for c in t
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if c.get("type").and_then(Value::as_str) == Some("rowNumber") {
                continue;
            }
            table.columns.push(ModelColumn {
                name: c.get("name")?.as_str()?.to_string(),
                data_type: c
                    .get("dataType")
                    .and_then(Value::as_str)
                    .unwrap_or("string")
                    .to_string(),
                expression: (c.get("type").and_then(Value::as_str) == Some("calculated"))
                    .then(|| expression_text(&c["expression"])),
            });
        }
        for m in t
            .get("measures")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(name) = m.get("name").and_then(Value::as_str) {
                table
                    .measures
                    .push((name.to_string(), expression_text(&m["expression"])));
            }
        }
        for p in t
            .get("partitions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let query = expression_text(&p["source"]["expression"]);
            if table.csv_path.is_none() {
                table.csv_path = csv_path_from_query(&query);
            }
        }
        out.tables.push(table);
    }
    for r in model
        .get("relationships")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let get = |k: &str| r.get(k).and_then(Value::as_str).map(str::to_string);
        if let (Some(a), Some(b), Some(c), Some(d)) = (
            get("fromTable"),
            get("fromColumn"),
            get("toTable"),
            get("toColumn"),
        ) {
            out.relationships.push(ModelRelationship {
                from_table: a,
                from_column: b,
                to_table: c,
                to_column: d,
            });
        }
    }
    Some(out)
}
