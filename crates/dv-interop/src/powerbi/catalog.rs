//! The catalog (`metadata.sqlitedb`) and the stored data files of a `.pbix` model
//! that holds its data.
//!
//! Every table is read from its CSV file, each column is encoded the way the
//! tabular engine stores it (see [`super::vertipaq`]) and the storage records
//! that describe those files are written next to the model's tables, columns,
//! measures and relationships.

use std::collections::HashMap;
use std::io::Read;

use rusqlite::{Connection, params_from_iter, types::Value};

use super::vertipaq::{self, FIRST_ID, Segment};
use super::{Model, ModelColumn, ModelTable, guid};
use crate::InteropError;

const SCHEMA: &str = include_str!("datamodel_schema.sql");

/// A fixed Windows FILETIME used for every "modified" stamp.
const STAMP: i64 = 134_399_581_609_984_148;
/// One segment holds at most this many rows.
const MAX_ROWS: usize = 8_388_608;
const ROW_NUMBER: &str = "RowNumber-2662979B-1795-4F74-8F37-6A1BA8059B61";

fn err(what: &str, e: impl std::fmt::Display) -> InteropError {
    InteropError::Format(format!("data model: {what}: {e}"))
}

fn int(v: i64) -> Value {
    Value::Integer(v)
}

fn text(v: &str) -> Value {
    Value::Text(v.to_string())
}

/// Inserts a row, giving every column not listed 0 (integer columns) or NULL.
fn insert(conn: &Connection, table: &str, values: &[(&str, Value)]) -> Result<(), InteropError> {
    let columns: Vec<(String, String)> = {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info(\"{table}\")"))
            .map_err(|e| err("schema", e))?;
        stmt.query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(|e| err("schema", e))?
            .collect::<Result<_, _>>()
            .map_err(|e| err("schema", e))?
    };
    for (name, _) in values {
        if !columns.iter().any(|(c, _)| c == name) {
            return Err(err(table, format!("no column {name}")));
        }
    }
    let row: Vec<Value> = columns
        .iter()
        .map(|(name, kind)| {
            values
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or(if kind.eq_ignore_ascii_case("INTEGER") {
                    Value::Integer(0)
                } else if kind.eq_ignore_ascii_case("REAL") {
                    Value::Real(0.0)
                } else {
                    Value::Null
                })
        })
        .collect();
    let names: Vec<String> = columns.iter().map(|(n, _)| format!("\"{n}\"")).collect();
    let marks = vec!["?"; columns.len()].join(",");
    conn.execute(
        &format!(
            "INSERT INTO \"{table}\" ({}) VALUES ({marks})",
            names.join(",")
        ),
        params_from_iter(row),
    )
    .map_err(|e| err(table, e))?;
    Ok(())
}

struct Ids(i64);

impl Ids {
    fn next(&mut self) -> i64 {
        self.0 += 1;
        self.0
    }
}

/// What the writer produced.
pub(super) struct Built {
    pub catalog: Vec<u8>,
    /// (path relative to the database folder, bytes)
    pub files: Vec<(String, Vec<u8>)>,
}

// ------------------------------------------------------------------ data

/// A column value, for matching relationship keys.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Int(i64),
    Real(u64),
    Text(String),
}

/// How a column is encoded.
#[derive(Debug)]
enum Encoding {
    /// A sorted dictionary of 64-bit integers.
    Longs(Vec<i64>),
    /// A dictionary of strings.
    Strings(Vec<String>),
    /// A dictionary of doubles.
    Doubles(Vec<f64>),
}

#[derive(Debug)]
struct EncodedColumn {
    name: String,
    data_type: String,
    encoding: Encoding,
    ids: Vec<u32>,
    distinct: usize,
    min_id: u32,
    max_id: u32,
    /// Whether the dictionary holds the empty string for blank cells.
    blank_entry: bool,
    /// The DAX expression of a calculated column.
    expression: Option<String>,
    /// Whether any row is blank (stored as the null id, 2).
    has_nulls: bool,
    /// The key of every data id (index = id); `None` for unused ids.
    id_keys: Vec<Option<Key>>,
    /// The key of every row.
    row_keys: Vec<Key>,
    min_text: String,
    max_text: String,
    longest: usize,
}

fn type_code(data_type: &str) -> i64 {
    match data_type {
        "int64" => 6,
        "double" => 8,
        "dateTime" => 9,
        "boolean" => 11,
        _ => 2,
    }
}

fn summarize_by(data_type: &str) -> i64 {
    match data_type {
        "int64" | "double" => 3,
        _ => 2,
    }
}

fn db_type(data_type: &str) -> i64 {
    match data_type {
        "int64" => 20,
        "double" => 5,
        "dateTime" => 7,
        "boolean" => 11,
        _ => 130,
    }
}

fn xm_type(data_type: &str) -> i64 {
    match data_type {
        "int64" | "boolean" => 0,
        "double" | "dateTime" => 1,
        _ => 2,
    }
}

/// Days since 1899-12-30 of a civil date.
fn ole_days(y: i64, m: i64, d: i64) -> i64 {
    fn from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
    from_civil(y, m, d) - from_civil(1899, 12, 30)
}

/// `YYYY-MM-DD`, optionally followed by a time, as days since 1899-12-30.
fn parse_date(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let m: i64 = s.get(5..7)?.parse().ok()?;
    let d: i64 = s.get(8..10)?.parse().ok()?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut days = ole_days(y, m, d) as f64;
    let rest = s.get(10..)?.trim_start_matches(['T', ' ']);
    if !rest.is_empty() {
        let mut parts = rest.trim_end_matches('Z').split(':');
        let h: f64 = parts.next()?.parse().ok()?;
        let mi: f64 = parts.next().map_or(Some(0.0), |p| p.parse().ok())?;
        let sec: f64 = parts.next().map_or(Some(0.0), |p| p.parse().ok())?;
        days += (h * 3600.0 + mi * 60.0 + sec) / 86_400.0;
    }
    Some(days)
}

/// Reads a CSV file into its header and rows.
fn read_csv(path: &str) -> Result<(Vec<String>, Vec<Vec<String>>), InteropError> {
    let mut raw = String::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_string(&mut raw))
        .map_err(|e| err(path, e))?;
    let mut lines = raw.trim_start_matches('\u{feff}').lines();
    let header =
        super::model::split_csv_line(lines.next().ok_or_else(|| err(path, "the file is empty"))?);
    let rows = lines
        .filter(|l| !l.trim().is_empty())
        .map(super::model::split_csv_line)
        .collect();
    Ok((header, rows))
}

fn first_seen<K: std::hash::Hash + Eq + Clone>(values: &[K]) -> (Vec<K>, Vec<u32>) {
    let mut seen: HashMap<K, u32> = HashMap::new();
    let mut order = Vec::new();
    let mut idx = Vec::with_capacity(values.len());
    for v in values {
        let next = order.len() as u32;
        let i = *seen.entry(v.clone()).or_insert_with(|| {
            order.push(v.clone());
            next
        });
        idx.push(i);
    }
    (order, idx)
}

fn bad(table: &str, column: &str, row: usize, what: &str, value: &str) -> InteropError {
    err(
        table,
        format!(
            "column '{column}', row {}: '{value}' is not {what}",
            row + 2
        ),
    )
}

fn encode_column(
    table: &str,
    column: &ModelColumn,
    cells: &[&str],
) -> Result<EncodedColumn, InteropError> {
    let blank = |c: &str| c.trim().is_empty();
    if column.data_type == "string" || !cells.iter().any(|c| blank(c)) {
        return encode_dense(table, column, cells);
    }
    // A blank number, date or boolean is stored as the reserved null id (2),
    // which the engine packs as 0 with the other ids shifted down by one. The
    // dictionary is built from the non-blank cells.
    let Some(filler) = cells.iter().find(|c| !blank(c)).copied() else {
        return Err(err(
            table,
            format!(
                "column '{}' holds only blank values, which a .pbix cannot store in a {} column",
                column.name, column.data_type
            ),
        ));
    };
    let filled: Vec<&str> = cells
        .iter()
        .map(|c| if blank(c) { filler } else { *c })
        .collect();
    let mut enc = encode_dense(table, column, &filled)?;
    for (row, cell) in cells.iter().enumerate() {
        if blank(cell) {
            enc.ids[row] = 2;
            // A key that matches nothing, so blanks never join to each other.
            enc.row_keys[row] = Key::Text(format!("\u{0}blank{row}"));
        }
    }
    enc.has_nulls = true;
    Ok(enc)
}

fn encode_dense(
    table: &str,
    column: &ModelColumn,
    cells: &[&str],
) -> Result<EncodedColumn, InteropError> {
    let name = column.name.clone();
    let blank = |c: &str| c.trim().is_empty();
    match column.data_type.as_str() {
        "int64" | "boolean" => {
            let mut values: Vec<i64> = Vec::with_capacity(cells.len());
            for (row, c) in cells.iter().enumerate() {
                let c = c.trim();
                values.push(if column.data_type == "boolean" {
                    match c.to_ascii_lowercase().as_str() {
                        "true" => 1,
                        "false" => 0,
                        _ => return Err(bad(table, &name, row, "true or false", c)),
                    }
                } else {
                    c.parse::<i64>()
                        .map_err(|_| bad(table, &name, row, "a whole number", c))?
                });
            }
            // A sorted dictionary of the distinct values: the engine takes a
            // column's minimum and maximum from its ends. (Value encoding, which
            // the engine uses for whole numbers, needs an attribute hierarchy to
            // answer them correctly.)
            let mut distinct_values = first_seen(&values).0;
            distinct_values.sort_unstable();
            let rank: HashMap<i64, u32> = distinct_values
                .iter()
                .enumerate()
                .map(|(i, v)| (*v, i as u32))
                .collect();
            let ids: Vec<u32> = values.iter().map(|v| rank[v] + FIRST_ID).collect();
            let max_id = FIRST_ID + distinct_values.len() as u32 - 1;
            let mut id_keys = vec![None; max_id as usize + 1];
            for (i, v) in distinct_values.iter().enumerate() {
                id_keys[FIRST_ID as usize + i] = Some(Key::Int(*v));
            }
            let min = distinct_values.first().copied().unwrap_or(0);
            let max = distinct_values.last().copied().unwrap_or(0);
            Ok(EncodedColumn {
                name,
                data_type: column.data_type.clone(),
                encoding: Encoding::Longs(distinct_values.clone()),
                ids,
                distinct: distinct_values.len(),
                min_id: FIRST_ID,
                max_id,
                blank_entry: false,
                expression: None,
                has_nulls: false,
                id_keys,
                row_keys: values.iter().map(|v| Key::Int(*v)).collect(),
                min_text: min.to_string(),
                max_text: max.to_string(),
                longest: 0,
            })
        }
        "double" | "dateTime" => {
            let mut values: Vec<f64> = Vec::with_capacity(cells.len());
            for (row, c) in cells.iter().enumerate() {
                let c = c.trim();
                let v = if column.data_type == "dateTime" {
                    parse_date(c).ok_or_else(|| bad(table, &name, row, "a date", c))?
                } else {
                    c.parse::<f64>()
                        .map_err(|_| bad(table, &name, row, "a number", c))?
                };
                if !v.is_finite() {
                    return Err(bad(table, &name, row, "a finite number", c));
                }
                values.push(v);
            }
            let present: Vec<u64> = values.iter().map(|v| v.to_bits()).collect();
            // Numeric dictionaries are sorted: the engine takes a column's
            // minimum and maximum from the ends of its dictionary.
            let mut order = first_seen(&present).0;
            order.sort_by(|a, b| f64::from_bits(*a).total_cmp(&f64::from_bits(*b)));
            let rank: HashMap<u64, u32> = order
                .iter()
                .enumerate()
                .map(|(i, b)| (*b, i as u32))
                .collect();
            let ids: Vec<u32> = present.iter().map(|b| rank[b] + FIRST_ID).collect();
            let distinct: Vec<f64> = order.iter().map(|b| f64::from_bits(*b)).collect();
            let max_id = FIRST_ID + distinct.len() as u32 - 1;
            let mut id_keys = vec![None; max_id as usize + 1];
            for (i, b) in order.iter().enumerate() {
                id_keys[FIRST_ID as usize + i] = Some(Key::Real(*b));
            }
            let lo = distinct.first().copied().unwrap_or(0.0);
            let hi = distinct.last().copied().unwrap_or(0.0);
            Ok(EncodedColumn {
                name,
                data_type: column.data_type.clone(),
                encoding: Encoding::Doubles(distinct.clone()),
                ids,
                distinct: distinct.len(),
                min_id: FIRST_ID,
                max_id,
                blank_entry: false,
                expression: None,
                has_nulls: false,
                id_keys,
                row_keys: present.iter().map(|b| Key::Real(*b)).collect(),
                min_text: lo.to_string(),
                max_text: hi.to_string(),
                longest: 0,
            })
        }
        _ => {
            // A blank cell is the empty string: an ordinary dictionary entry.
            let values: Vec<String> = cells
                .iter()
                .map(|c| {
                    if blank(c) {
                        String::new()
                    } else {
                        (*c).to_string()
                    }
                })
                .collect();
            let blank_entry = values.iter().any(String::is_empty);
            let (order, idx) = first_seen(&values);
            let ids: Vec<u32> = idx.iter().map(|i| i + FIRST_ID).collect();
            let max_id = FIRST_ID + order.len() as u32 - 1;
            let mut id_keys = vec![None; max_id as usize + 1];
            for (i, s) in order.iter().enumerate() {
                id_keys[FIRST_ID as usize + i] = Some(Key::Text(s.clone()));
            }
            let longest = order
                .iter()
                .map(|s| s.encode_utf16().count())
                .max()
                .unwrap_or(0);
            let min_text = order.iter().min().cloned().unwrap_or_default();
            let max_text = order.iter().max().cloned().unwrap_or_default();
            Ok(EncodedColumn {
                name,
                data_type: column.data_type.clone(),
                distinct: order.len(),
                encoding: Encoding::Strings(order),
                ids,
                min_id: FIRST_ID,
                max_id,
                blank_entry,
                expression: None,
                has_nulls: false,
                id_keys,
                row_keys: values.into_iter().map(Key::Text).collect(),
                min_text,
                max_text,
                longest,
            })
        }
    }
}

struct TableData {
    rows: usize,
    columns: Vec<EncodedColumn>,
}

/// The type of a column from its cells, as the CSV schema sniffer would pick it.
fn infer_type(cells: &[&str]) -> &'static str {
    let mut kind: Option<&'static str> = None;
    for c in cells.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
        let this = if c.parse::<i64>().is_ok() {
            "int64"
        } else if c.parse::<f64>().is_ok() {
            "double"
        } else if parse_date(c).is_some() && c.len() >= 10 {
            "dateTime"
        } else if c.eq_ignore_ascii_case("true") || c.eq_ignore_ascii_case("false") {
            "boolean"
        } else {
            "string"
        };
        kind = Some(match (kind, this) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some("int64"), "double") | (Some("double"), "int64") => "double",
            _ => "string",
        });
    }
    kind.unwrap_or("string")
}

/// The values of calculated columns, by (table, column).
type CalculatedValues = HashMap<(String, String), Vec<String>>;

fn load_table(table: &ModelTable, values: &CalculatedValues) -> Result<TableData, InteropError> {
    let path = table
        .csv_path
        .as_deref()
        .ok_or_else(|| err(&table.name, "the table has no CSV file to load"))?;
    let (header, rows) = read_csv(path)?;
    if rows.is_empty() {
        return Err(err(&table.name, "the CSV file has no data rows"));
    }
    if rows.len() > MAX_ROWS {
        return Err(err(
            &table.name,
            format!("more than {MAX_ROWS} rows do not fit one segment"),
        ));
    }
    let mut columns = Vec::new();
    for column in &table.columns {
        if let Some((_, sources)) = table.key_columns.iter().find(|(n, _)| *n == column.name) {
            let at: Vec<usize> = sources
                .iter()
                .map(|s| {
                    header
                        .iter()
                        .position(|h| h == s)
                        .ok_or_else(|| err(&table.name, format!("no column '{s}' in the CSV")))
                })
                .collect::<Result<_, _>>()?;
            let joined: Vec<String> = rows
                .iter()
                .map(|r| {
                    at.iter()
                        .map(|i| r.get(*i).map_or("", String::as_str))
                        .collect::<Vec<_>>()
                        .join("|")
                })
                .collect();
            let cells: Vec<&str> = joined.iter().map(String::as_str).collect();
            let typed = ModelColumn {
                name: column.name.clone(),
                data_type: "string".into(),
                expression: None,
            };
            let mut encoded = encode_column(&table.name, &typed, &cells)?;
            encoded.expression = column.expression.clone();
            columns.push(encoded);
            continue;
        }
        if let Some(expression) = &column.expression {
            let cells = values
                .get(&(table.name.clone(), column.name.clone()))
                .ok_or_else(|| {
                    err(
                        &table.name,
                        format!(
                            "no values were computed for calculated column '{}'",
                            column.name
                        ),
                    )
                })?;
            if cells.len() != rows.len() {
                return Err(err(
                    &table.name,
                    format!(
                        "calculated column '{}' has {} values for {} rows",
                        column.name,
                        cells.len(),
                        rows.len()
                    ),
                ));
            }
            let cells: Vec<&str> = cells.iter().map(String::as_str).collect();
            let typed = ModelColumn {
                name: column.name.clone(),
                data_type: infer_type(&cells).to_string(),
                expression: None,
            };
            let mut encoded = encode_column(&table.name, &typed, &cells)?;
            encoded.expression = Some(expression.clone());
            columns.push(encoded);
            continue;
        }
        let at = header
            .iter()
            .position(|h| h == &column.name)
            .ok_or_else(|| {
                err(
                    &table.name,
                    format!("no column '{}' in the CSV", column.name),
                )
            })?;
        let cells: Vec<&str> = rows
            .iter()
            .map(|r| r.get(at).map_or("", String::as_str))
            .collect();
        columns.push(encode_column(&table.name, column, &cells)?);
    }
    Ok(TableData {
        rows: rows.len(),
        columns,
    })
}

// --------------------------------------------------------------- catalog

/// File and folder names: anything but letters, digits and spaces becomes a space.
fn clean(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect()
}

/// Everything about a table that relationships need later.
struct TableOut {
    name: String,
    id: i64,
    columns: Vec<(String, i64)>,
    data: TableData,
}

struct Writer<'a> {
    conn: &'a Connection,
    ids: Ids,
    files: Vec<(String, Vec<u8>)>,
}

impl Writer<'_> {
    fn folder(&mut self, owner: i64, owner_type: i64, path: &str) -> Result<i64, InteropError> {
        let id = self.ids.next();
        insert(
            self.conn,
            "StorageFolder",
            &[
                ("ID", int(id)),
                ("OwnerID", int(owner)),
                ("OwnerType", int(owner_type)),
                ("Path", text(path)),
            ],
        )?;
        Ok(id)
    }

    /// Stores a file and its catalog record; returns the record's id.
    fn file(
        &mut self,
        owner: i64,
        owner_type: i64,
        folder: i64,
        folder_path: &str,
        name: &str,
        bytes: Vec<u8>,
    ) -> Result<i64, InteropError> {
        let id = self.ids.next();
        insert(
            self.conn,
            "StorageFile",
            &[
                ("ID", int(id)),
                ("OwnerID", int(owner)),
                ("OwnerType", int(owner_type)),
                ("StorageFolderID", int(folder)),
                ("FileName", text(name)),
            ],
        )?;
        self.files.push((format!("{folder_path}\\{name}"), bytes));
        Ok(id)
    }
}

pub(super) fn build(
    model: &Model,
    queries: &[(String, Vec<String>)],
    values: &CalculatedValues,
) -> Result<Built, InteropError> {
    let tables: Vec<TableData> = model
        .tables
        .iter()
        .map(|t| load_table(t, values))
        .collect::<Result<_, _>>()?;
    let file = tempfile::NamedTempFile::new().map_err(|e| err("temporary file", e))?;
    let files = {
        let conn = Connection::open(file.path()).map_err(|e| err("catalog", e))?;
        conn.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=OFF;")
            .map_err(|e| err("catalog", e))?;
        conn.execute_batch(SCHEMA).map_err(|e| err("schema", e))?;
        let files = fill(&conn, model, queries, tables)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
        files
    };
    let mut catalog = Vec::new();
    std::fs::File::open(file.path())
        .and_then(|mut f| f.read_to_end(&mut catalog))
        .map_err(|e| err("catalog", e))?;
    Ok(Built { catalog, files })
}

fn fill(
    conn: &Connection,
    model: &Model,
    queries: &[(String, Vec<String>)],
    data: Vec<TableData>,
) -> Result<Vec<(String, Vec<u8>)>, InteropError> {
    let mut w = Writer {
        conn,
        ids: Ids(0),
        files: Vec::new(),
    };
    let model_id = w.ids.next();
    insert(
        conn,
        "Model",
        &[
            ("ID", int(model_id)),
            ("Name", text("Model")),
            ("Culture", text("en-US")),
            ("ModifiedTime", int(STAMP)),
            ("StructureModifiedTime", int(STAMP)),
            (
                "DataAccessOptions",
                text("{\n  \"legacyRedirects\": true,\n  \"returnErrorValuesAsNull\": true\n}"),
            ),
            ("DefaultPowerBIDataSourceVersion", int(2)),
            ("DataSourceDefaultMaxConnections", int(10)),
            ("SourceQueryCulture", text("en-US")),
            ("DisableAutoExists", int(-1)),
            ("MaxParallelismPerRefresh", int(-1)),
        ],
    )?;
    let culture = w.ids.next();
    let linguistic = w.ids.next();
    insert(
        conn,
        "Culture",
        &[
            ("ID", int(culture)),
            ("ModelID", int(model_id)),
            ("Name", text("en-US")),
            ("LinguisticMetadataID", int(linguistic)),
            ("ModifiedTime", int(STAMP)),
            ("StructureModifiedTime", int(STAMP)),
        ],
    )?;
    insert(
        conn,
        "LinguisticMetadata",
        &[
            ("ID", int(linguistic)),
            ("CultureID", int(culture)),
            (
                "Content",
                text("{\"Version\":\"1.0.0\",\"Language\":\"en-US\"}"),
            ),
            ("ModifiedTime", int(STAMP)),
            ("ContentType", int(1)),
        ],
    )?;

    let mut outs: Vec<TableOut> = Vec::new();
    let mut order = Vec::new();
    for (table, data) in model.tables.iter().zip(data) {
        order.push(format!("\"{}\"", table.name.replace('"', "\\\"")));
        let out = write_table(&mut w, model_id, table, queries, data)?;
        outs.push(out);
    }

    for rel in &model.relationships {
        write_relationship(&mut w, model_id, &outs, rel)?;
    }

    for (property, value) in [
        ("__PBI_TimeIntelligenceEnabled", "0".to_string()),
        ("PBI_QueryOrder", format!("[{}]", order.join(","))),
    ] {
        let id = w.ids.next();
        insert(
            conn,
            "Annotation",
            &[
                ("ID", int(id)),
                ("ObjectID", int(model_id)),
                ("ObjectType", int(1)),
                ("Name", text(property)),
                ("Value", text(&value)),
                ("ModifiedTime", int(STAMP)),
            ],
        )?;
    }

    let max_id = w.ids.0 + 1;
    for (property, value) in [
        ("MAXID", Some(max_id)),
        ("SCHEMAVERSION", Some(111)),
        ("LASTSCHEMAVERSION", Some(111)),
        ("DATABASEBLOCKINGSTATE", None),
        ("RESTOREABFSOURCE", None),
        ("LASTCUBEDATACHANGE", Some(STAMP)),
        ("LASTCUBESCHEMACHANGE", Some(STAMP)),
        ("DATASOURCEVERSION", Some(2)),
        ("FEATURESTOUPGRADE", None),
        ("LASTCOMMITTED", Some(STAMP)),
        ("LASTMETADATACHANGEFORSYNCCOMMITTED", Some(STAMP)),
        ("LASTPBIPUBLICXMLACOMMITTEDTIME", None),
    ] {
        conn.execute(
            "INSERT INTO DBPROPERTIES (NAME, VALUE) VALUES (?, ?)",
            rusqlite::params![property, value],
        )
        .map_err(|e| err("DBPROPERTIES", e))?;
    }
    Ok(w.files)
}

fn write_table(
    w: &mut Writer<'_>,
    model_id: i64,
    table: &ModelTable,
    queries: &[(String, Vec<String>)],
    data: TableData,
) -> Result<TableOut, InteropError> {
    let conn = w.conn;
    let table_id = w.ids.next();
    let table_storage = w.ids.next();
    let sname = clean(&table.name);
    let table_dir = format!("{sname} ({table_id}).tbl");
    insert(
        conn,
        "Table",
        &[
            ("ID", int(table_id)),
            ("ModelID", int(model_id)),
            ("Name", text(&table.name)),
            ("TableStorageID", int(table_storage)),
            ("ModifiedTime", int(STAMP)),
            ("StructureModifiedTime", int(STAMP)),
            ("LineageTag", text(&guid(&format!("table:{}", table.name)))),
        ],
    )?;
    let table_folder = w.folder(table_storage, 18, &table_dir)?;
    insert(
        conn,
        "TableStorage",
        &[
            ("ID", int(table_storage)),
            ("TableID", int(table_id)),
            ("Name", text(&format!("{sname} ({table_id})"))),
            ("Version", int(1)),
            ("Settings", int(4353)),
            ("StorageFolderID", int(table_folder)),
        ],
    )?;

    // The partition comes first so its folder exists for the column files.
    let partition = w.ids.next();
    let partition_storage = w.ids.next();
    let partition_dir = format!("{table_dir}\\{partition}.prt");
    let query = queries
        .iter()
        .find(|(t, _)| *t == table.name)
        .map(|(_, lines)| lines.join("\n"));
    insert(
        conn,
        "Partition",
        &[
            ("ID", int(partition)),
            ("TableID", int(table_id)),
            ("Name", text(&table.name)),
            (
                "QueryDefinition",
                query.as_deref().map_or(Value::Null, text),
            ),
            ("State", int(1)),
            ("Type", int(4)),
            ("PartitionStorageID", int(partition_storage)),
            ("DataView", int(3)),
            ("ModifiedTime", int(STAMP)),
            ("RefreshedTime", int(STAMP)),
            ("RangeGranularity", int(-1)),
        ],
    )?;
    let partition_folder = w.folder(partition_storage, 20, &partition_dir)?;
    let segment_map = w.ids.next();
    insert(
        conn,
        "PartitionStorage",
        &[
            ("ID", int(partition_storage)),
            ("PartitionID", int(partition)),
            ("Name", text(&format!("{sname} ({partition})"))),
            ("SegmentMapStorageID", int(segment_map)),
            ("StorageFolderID", int(partition_folder)),
        ],
    )?;
    insert(
        conn,
        "SegmentMapStorage",
        &[
            ("ID", int(segment_map)),
            ("PartitionStorageID", int(partition_storage)),
            ("Type", int(3)),
            ("RecordCount", int(data.rows as i64)),
            ("SegmentCount", int(1)),
            ("RecordsPerSegment", int(data.rows as i64)),
        ],
    )?;

    let mut column_ids = Vec::new();
    let all: Vec<Option<&EncodedColumn>> = std::iter::once(None)
        .chain(data.columns.iter().map(Some))
        .collect();
    for (position, column) in all.into_iter().enumerate() {
        let column_id = w.ids.next();
        let hierarchy = w.ids.next();
        let column_storage = w.ids.next();
        let dictionary = w.ids.next();
        let hierarchy_storage = w.ids.next();
        let partition_storage_row = w.ids.next();
        let segment_storage = w.ids.next();
        let (name, data_type) = column.map_or((ROW_NUMBER, "int64"), |c| {
            (c.name.as_str(), c.data_type.as_str())
        });
        let hidden = column.is_none();
        let calculated = column.and_then(|c| c.expression.as_deref());
        let scol = clean(name);
        insert(
            conn,
            "Column",
            &[
                ("ID", int(column_id)),
                ("TableID", int(table_id)),
                ("ExplicitName", text(name)),
                ("ExplicitDataType", int(type_code(data_type))),
                ("InferredDataType", int(19)),
                ("IsHidden", int(i64::from(hidden))),
                ("State", int(1)),
                ("IsUnique", int(i64::from(hidden))),
                ("IsKey", int(i64::from(hidden))),
                ("IsNullable", int(i64::from(!hidden))),
                ("Alignment", int(1)),
                ("TableDetailPosition", int(-1)),
                (
                    "SummarizeBy",
                    int(if hidden { 1 } else { summarize_by(data_type) }),
                ),
                ("ColumnStorageID", int(column_storage)),
                (
                    "Type",
                    int(if hidden {
                        3
                    } else if calculated.is_some() {
                        2
                    } else {
                        1
                    }),
                ),
                (
                    "SourceColumn",
                    if hidden || calculated.is_some() {
                        Value::Null
                    } else {
                        text(name)
                    },
                ),
                ("Expression", calculated.map_or(Value::Null, text)),
                ("IsAvailableInMDX", int(1)),
                ("AttributeHierarchyID", int(hierarchy)),
                ("ModifiedTime", int(STAMP)),
                ("StructureModifiedTime", int(STAMP)),
                ("RefreshedTime", int(31_240_512_000_000_000)),
                (
                    "LineageTag",
                    if hidden {
                        Value::Null
                    } else {
                        text(&guid(&format!("column:{}:{name}", table.name)))
                    },
                ),
                ("EvaluationBehavior", int(1)),
            ],
        )?;

        let (records, distinct, min_id, max_id, min_text, max_text, longest) = match column {
            None => (
                data.rows,
                data.rows,
                FIRST_ID,
                FIRST_ID + data.rows as u32 - 1,
                "0".to_string(),
                (data.rows - 1).to_string(),
                0,
            ),
            Some(c) => (
                data.rows,
                c.distinct,
                c.min_id,
                c.max_id,
                c.min_text.clone(),
                c.max_text.clone(),
                c.longest,
            ),
        };
        let value_encoded = column.is_none();
        let segment = Segment {
            records,
            min_id,
            max_id,
            orig_min: if value_encoded { 3 } else { 2 },
            base_id: u64::from(!hidden),
            ids: column.map(|c| c.ids.clone()),
            has_nulls: column.is_some_and(|c| c.has_nulls),
        };

        insert(
            conn,
            "AttributeHierarchy",
            &[
                ("ID", int(hierarchy)),
                ("ColumnID", int(column_id)),
                ("State", int(1)),
                ("AttributeHierarchyStorageID", int(hierarchy_storage)),
                ("ModifiedTime", int(STAMP)),
                ("RefreshedTime", int(STAMP)),
            ],
        )?;
        insert(
            conn,
            "AttributeHierarchyStorage",
            &[
                ("ID", int(hierarchy_storage)),
                ("AttributeHierarchyID", int(hierarchy)),
                ("MaterializationType", int(3)),
                ("ColumnPositionToData", int(-1)),
                ("ColumnDataToPosition", int(-1)),
                ("DistinctDataCount", int(distinct as i64)),
                ("DataVersion", int(1)),
                ("HasStatistics", int(1)),
                ("MinValue", text(&min_text)),
                ("MaxValue", text(&max_text)),
                ("StringValueMaxLength", int(longest as i64)),
            ],
        )?;
        insert(
            conn,
            "ColumnStorage",
            &[
                ("ID", int(column_storage)),
                ("ColumnID", int(column_id)),
                ("Name", text(&format!("{scol} ({column_id})"))),
                ("StoragePosition", int(position as i64)),
                ("DictionaryStorageID", int(dictionary)),
                ("Settings", int(1)),
                ("ColumnFlags", int(if hidden { 31 } else { 8 })),
                ("Locale", int(1033)),
                ("Statistics_DistinctStates", int(distinct as i64)),
                ("Statistics_MinDataID", int(i64::from(min_id))),
                ("Statistics_MaxDataID", int(i64::from(max_id))),
                ("Statistics_OriginalMinSegmentDataID", int(2)),
                ("Statistics_HasNulls", int(i64::from(segment.has_nulls))),
                ("Statistics_RLESortOrder", int(-1)),
                ("Statistics_RowCount", int(records as i64)),
                ("Statistics_OthersRLERuns", int(i64::from(!hidden))),
                ("Statistics_Usage", int(3)),
                ("Statistics_DBType", int(db_type(data_type))),
                ("Statistics_XMType", int(xm_type(data_type))),
                ("Statistics_EncodingHint", int(i64::from(hidden))),
                ("DeltaColumnMappingId", int(-1)),
            ],
        )?;

        // Dictionary record and (for dictionary columns) its file.
        let version = 1;
        let tbl = format!("{sname} ({table_id})");
        let stem = format!("{version}.{tbl}.{scol} ({column_id})");
        let (dict_type, dict_data_type, base, magnitude, last_id, nullable, flags, dict_file) =
            match column.map(|c| &c.encoding) {
                None => (2, 6, -3, 1.0, 2, 1, 0, None),
                Some(Encoding::Longs(values)) => (
                    1,
                    type_code(data_type),
                    2,
                    0.0,
                    2 + values.len() as i64,
                    i64::from(column.is_some_and(|c| c.has_nulls)),
                    0,
                    Some(vertipaq::long_dictionary(values)),
                ),
                Some(Encoding::Strings(strings)) => (
                    1,
                    2,
                    2,
                    0.0,
                    2 + strings.len() as i64,
                    i64::from(false),
                    3,
                    Some(vertipaq::string_dictionary(
                        strings,
                        column.is_some_and(|c| c.blank_entry),
                    )),
                ),
                Some(Encoding::Doubles(values)) => (
                    1,
                    8,
                    2,
                    0.0,
                    2 + values.len() as i64,
                    i64::from(column.is_some_and(|c| c.has_nulls)),
                    0,
                    Some(vertipaq::number_dictionary(values)),
                ),
            };
        let dict_size = dict_file.as_ref().map_or(128, |f| f.len() as i64);
        let dict_file_id = match dict_file {
            Some(bytes) => w.file(
                dictionary,
                22,
                table_folder,
                &table_dir,
                &format!("{stem}.dictionary"),
                bytes,
            )?,
            None => 0,
        };
        insert(
            conn,
            "DictionaryStorage",
            &[
                ("ID", int(dictionary)),
                ("ColumnStorageID", int(column_storage)),
                ("Type", int(dict_type)),
                ("DataType", int(dict_data_type)),
                ("DataVersion", int(1)),
                ("BaseId", int(base)),
                ("Magnitude", Value::Real(magnitude)),
                ("LastId", int(last_id)),
                ("IsNullable", int(nullable)),
                ("DictionaryFlags", int(flags)),
                ("StorageFileID", int(dict_file_id)),
                ("Size", int(dict_size)),
            ],
        )?;

        // The segment files.
        let idf_id = w.file(
            partition_storage_row,
            23,
            partition_folder,
            &partition_dir,
            &format!("{stem}.0.idf"),
            vertipaq::idf(&segment),
        )?;
        let meta_id = w.file(
            segment_storage,
            24,
            partition_folder,
            &partition_dir,
            &format!("{stem}.0.idfmeta"),
            vertipaq::idfmeta(&segment),
        )?;
        insert(
            conn,
            "ColumnPartitionStorage",
            &[
                ("ID", int(partition_storage_row)),
                ("ColumnStorageID", int(column_storage)),
                ("PartitionStorageID", int(partition_storage)),
                ("DataVersion", int(1)),
                ("State", int(1)),
                ("SegmentStorageID", int(segment_storage)),
                ("StorageFileID", int(idf_id)),
            ],
        )?;
        insert(
            conn,
            "SegmentStorage",
            &[
                ("ID", int(segment_storage)),
                ("ColumnPartitionStorageID", int(partition_storage_row)),
                ("SegmentCount", int(1)),
                ("StorageFileID", int(meta_id)),
            ],
        )?;
        column_ids.push((name.to_string(), column_id));
    }

    for (measure, expression) in &table.measures {
        let id = w.ids.next();
        insert(
            conn,
            "Measure",
            &[
                ("ID", int(id)),
                ("TableID", int(table_id)),
                ("Name", text(measure)),
                ("DataType", int(8)),
                ("Expression", text(expression)),
                ("State", int(1)),
                ("ModifiedTime", int(STAMP)),
                ("StructureModifiedTime", int(STAMP)),
                (
                    "LineageTag",
                    text(&guid(&format!("measure:{}:{measure}", table.name))),
                ),
            ],
        )?;
    }
    Ok(TableOut {
        name: table.name.clone(),
        id: table_id,
        columns: column_ids,
        data,
    })
}

fn write_relationship(
    w: &mut Writer<'_>,
    model_id: i64,
    tables: &[TableOut],
    rel: &super::ModelRelationship,
) -> Result<(), InteropError> {
    let find = |t: &str, c: &str| -> Option<(&TableOut, i64, &EncodedColumn)> {
        let table = tables.iter().find(|x| x.name == t)?;
        let (_, column_id) = table.columns.iter().find(|(n, _)| n == c)?;
        let column = table.data.columns.iter().find(|x| x.name == c)?;
        Some((table, *column_id, column))
    };
    let (Some((from_table, from_column, from)), Some((to_table, to_column, to))) = (
        find(&rel.from_table, &rel.from_column),
        find(&rel.to_table, &rel.to_column),
    ) else {
        return Ok(());
    };

    // For every data id of the "from" column: the row of the matching key in
    // the "to" table, plus one (zero when there is none).
    let mut first_row: HashMap<&Key, u32> = HashMap::new();
    for (row, key) in to.row_keys.iter().enumerate() {
        first_row.entry(key).or_insert(row as u32 + 1);
    }
    let positions: Vec<u32> = from
        .id_keys
        .iter()
        .map(|key| {
            key.as_ref()
                .and_then(|k| first_row.get(k).copied())
                .unwrap_or(0)
        })
        .collect();
    let records = positions.len();
    let (index_idf, class) = vertipaq::index_idf(&positions);
    let words = u64::from_le_bytes(index_idf[..8].try_into().unwrap_or([0; 8])) as usize;
    let index_meta = vertipaq::index_idfmeta(records, class, words);

    let conn = w.conn;
    let rel_id = w.ids.next();
    let rel_storage = w.ids.next();
    let rel_index = w.ids.next();
    let rel_name = guid(&format!("relationship:{}:{}", rel.from_table, rel.to_table));

    // The hidden "R$" table that holds the index.
    let system_name = format!(
        "R${} ({})${} ({rel_id})",
        clean(&from_table.name),
        from_table.id,
        rel_name.replace('-', " ")
    );
    let table_id = w.ids.next();
    let table_storage = w.ids.next();
    let partition = w.ids.next();
    let partition_storage = w.ids.next();
    let segment_map = w.ids.next();
    let column_id = w.ids.next();
    let column_storage = w.ids.next();
    let dictionary = w.ids.next();
    let column_partition = w.ids.next();
    let segment_storage = w.ids.next();
    let table_dir = format!("{system_name}$({table_id}).tbl");
    let partition_dir = format!("{table_dir}\\{partition}.prt");
    insert(
        conn,
        "Table",
        &[
            ("ID", int(table_id)),
            // System tables belong to no model: with the model's id the
            // engine cannot find them when it compiles the model.
            ("ModelID", int(0)),
            ("Name", text(&system_name)),
            ("IsHidden", int(1)),
            ("TableStorageID", int(table_storage)),
            ("ModifiedTime", int(STAMP)),
            ("StructureModifiedTime", int(STAMP)),
            ("SystemFlags", int(1)),
        ],
    )?;
    insert(
        conn,
        "Column",
        &[
            ("ID", int(column_id)),
            ("TableID", int(table_id)),
            ("ExplicitName", text("INDEX")),
            ("ExplicitDataType", int(6)),
            ("InferredDataType", int(19)),
            ("State", int(1)),
            ("IsNullable", int(1)),
            ("Alignment", int(1)),
            ("TableDetailPosition", int(-1)),
            ("SummarizeBy", int(1)),
            ("ColumnStorageID", int(column_storage)),
            ("Type", int(1)),
            ("IsAvailableInMDX", int(1)),
            ("ModifiedTime", int(STAMP)),
            ("StructureModifiedTime", int(STAMP)),
            ("RefreshedTime", int(31_240_512_000_000_000)),
            ("SystemFlags", int(1)),
            ("EvaluationBehavior", int(1)),
        ],
    )?;
    insert(
        conn,
        "Partition",
        &[
            ("ID", int(partition)),
            ("TableID", int(table_id)),
            ("Name", text(&system_name)),
            ("State", int(1)),
            ("Type", int(3)),
            ("PartitionStorageID", int(partition_storage)),
            ("Mode", int(2)),
            ("DataView", int(3)),
            ("ModifiedTime", int(STAMP)),
            ("RefreshedTime", int(31_240_512_000_000_000)),
            ("SystemFlags", int(1)),
            ("RangeGranularity", int(-1)),
        ],
    )?;
    let table_folder = w.folder(table_storage, 18, &table_dir)?;
    insert(
        conn,
        "TableStorage",
        &[
            ("ID", int(table_storage)),
            ("TableID", int(table_id)),
            ("Name", text(&system_name)),
            ("Settings", int(2)),
            ("StorageFolderID", int(table_folder)),
        ],
    )?;
    let partition_folder = w.folder(partition_storage, 20, &partition_dir)?;
    insert(
        conn,
        "PartitionStorage",
        &[
            ("ID", int(partition_storage)),
            ("PartitionID", int(partition)),
            ("Name", text(&system_name)),
            ("SegmentMapStorageID", int(segment_map)),
            ("StorageFolderID", int(partition_folder)),
        ],
    )?;
    insert(
        conn,
        "SegmentMapStorage",
        &[
            ("ID", int(segment_map)),
            ("PartitionStorageID", int(partition_storage)),
            ("Type", int(3)),
            ("RecordCount", int(records as i64)),
            ("SegmentCount", int(1)),
            ("RecordsPerSegment", int(records as i64)),
        ],
    )?;
    insert(
        conn,
        "ColumnStorage",
        &[
            ("ID", int(column_storage)),
            ("ColumnID", int(column_id)),
            ("Name", text("INDEX")),
            ("DictionaryStorageID", int(dictionary)),
            ("Settings", int(3)),
            ("Statistics_DistinctStates", int(1)),
            ("Statistics_MinDataID", int(2)),
            ("Statistics_MaxDataID", int(2)),
            ("Statistics_OriginalMinSegmentDataID", int(2)),
            ("Statistics_RLESortOrder", int(-1)),
            ("Statistics_Usage", int(3)),
            ("DeltaColumnMappingId", int(-1)),
        ],
    )?;
    insert(
        conn,
        "DictionaryStorage",
        &[
            ("ID", int(dictionary)),
            ("ColumnStorageID", int(column_storage)),
            ("DataType", int(19)),
        ],
    )?;
    let stem = format!("0.{system_name}.INDEX");
    let idf_id = w.file(
        column_partition,
        23,
        partition_folder,
        &partition_dir,
        &format!("{stem}.0.idf"),
        index_idf,
    )?;
    let meta_id = w.file(
        segment_storage,
        24,
        partition_folder,
        &partition_dir,
        &format!("{stem}.0.idfmeta"),
        index_meta,
    )?;
    insert(
        conn,
        "ColumnPartitionStorage",
        &[
            ("ID", int(column_partition)),
            ("ColumnStorageID", int(column_storage)),
            ("PartitionStorageID", int(partition_storage)),
            ("State", int(3)),
            ("SegmentStorageID", int(segment_storage)),
            ("StorageFileID", int(idf_id)),
        ],
    )?;
    insert(
        conn,
        "SegmentStorage",
        &[
            ("ID", int(segment_storage)),
            ("ColumnPartitionStorageID", int(column_partition)),
            ("SegmentCount", int(1)),
            ("StorageFileID", int(meta_id)),
        ],
    )?;

    insert(
        conn,
        "Relationship",
        &[
            ("ID", int(rel_id)),
            ("ModelID", int(model_id)),
            ("Name", text(&rel_name)),
            ("IsActive", int(1)),
            ("Type", int(1)),
            ("CrossFilteringBehavior", int(1)),
            ("JoinOnDateBehavior", int(1)),
            ("FromTableID", int(from_table.id)),
            ("FromColumnID", int(from_column)),
            ("FromCardinality", int(2)),
            ("ToTableID", int(to_table.id)),
            ("ToColumnID", int(to_column)),
            ("ToCardinality", int(1)),
            ("State", int(1)),
            ("RelationshipStorageID", int(rel_storage)),
            ("ModifiedTime", int(STAMP)),
            ("RefreshedTime", int(STAMP)),
            ("SecurityFilteringBehavior", int(1)),
        ],
    )?;
    insert(
        conn,
        "RelationshipStorage",
        &[
            ("ID", int(rel_storage)),
            ("RelationshipID", int(rel_id)),
            (
                "Name",
                text(&format!("{} ({rel_id})", rel_name.replace('-', " "))),
            ),
            ("RelationshipIndexStorageID", int(rel_index)),
        ],
    )?;
    insert(
        conn,
        "RelationshipIndexStorage",
        &[
            ("ID", int(rel_index)),
            ("RelationshipStorageID", int(rel_storage)),
            ("IndexType", int(1)),
            ("RecordCount", int(records as i64)),
            ("SystemTableID", int(table_id)),
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, data_type: &str) -> ModelColumn {
        ModelColumn {
            name: name.into(),
            data_type: data_type.into(),
            expression: None,
        }
    }

    fn encode(data_type: &str, cells: &[&str]) -> Result<EncodedColumn, InteropError> {
        encode_column("t", &column("c", data_type), cells)
    }

    #[test]
    fn dates_count_days_from_1899_12_30() {
        assert_eq!(parse_date("1899-12-30"), Some(0.0));
        assert_eq!(parse_date("2020-01-01"), Some(43831.0));
        assert_eq!(parse_date("2020-01-01 12:00:00"), Some(43831.5));
        assert_eq!(parse_date("2020-01-01T06:00"), Some(43831.25));
        assert_eq!(parse_date("2020-13-01"), None);
        assert_eq!(parse_date("yesterday"), None);
    }

    #[test]
    fn whole_numbers_get_a_sorted_dictionary() {
        let c = encode("int64", &["5", "1", "5", "3"]).unwrap();
        assert!(matches!(&c.encoding, Encoding::Longs(v) if v == &[1, 3, 5]));
        assert_eq!(c.ids, vec![5, 3, 5, 4]);
        assert_eq!((c.min_id, c.max_id, c.distinct), (3, 5, 3));
        assert_eq!((c.min_text.as_str(), c.max_text.as_str()), ("1", "5"));
    }

    #[test]
    fn decimals_and_dates_get_a_sorted_dictionary() {
        let c = encode("double", &["2.5", "1.5", "2.5"]).unwrap();
        assert!(matches!(&c.encoding, Encoding::Doubles(v) if v == &[1.5, 2.5]));
        assert_eq!(c.ids, vec![4, 3, 4]);
        let d = encode("dateTime", &["2021-05-02", "2020-01-01"]).unwrap();
        assert!(matches!(&d.encoding, Encoding::Doubles(v) if v[0] < v[1]));
        assert_eq!(d.ids, vec![4, 3]);
    }

    #[test]
    fn strings_keep_their_first_appearance_order() {
        let c = encode("string", &["b", "a", "b"]).unwrap();
        assert!(matches!(&c.encoding, Encoding::Strings(v) if v == &["b", "a"]));
        assert_eq!(c.ids, vec![3, 4, 3]);
        assert_eq!(c.longest, 1);
    }

    #[test]
    fn blank_text_is_an_empty_dictionary_entry() {
        let c = encode("string", &["a", " ", "", "a"]).unwrap();
        assert!(matches!(&c.encoding, Encoding::Strings(v) if v == &["a", ""]));
        assert_eq!(c.ids, vec![3, 4, 4, 3]);
        assert!(c.blank_entry);
        assert!(!encode("string", &["a", "b"]).unwrap().blank_entry);
    }

    #[test]
    fn booleans_are_true_or_false() {
        let c = encode("boolean", &["true", "FALSE", "true"]).unwrap();
        assert!(matches!(&c.encoding, Encoding::Longs(v) if v == &[0, 1]));
        assert!(encode("boolean", &["maybe"]).is_err());
    }

    #[test]
    fn blank_numbers_use_the_null_id() {
        let c = encode("int64", &["10", "", "30", " "]).unwrap();
        assert!(c.has_nulls);
        assert!(matches!(&c.encoding, Encoding::Longs(v) if v == &[10, 30]));
        assert_eq!(c.ids, vec![3, 2, 4, 2]);
        assert_eq!((c.min_id, c.max_id, c.distinct), (3, 4, 2));
        for (kind, value) in [("double", "1.5"), ("dateTime", "2024-01-01")] {
            let c = encode(kind, &["", value]).unwrap();
            assert!(c.has_nulls && c.ids == vec![2, 3], "{kind}");
        }
        assert!(!encode("int64", &["1", "2"]).unwrap().has_nulls);
        // A blank never joins to another blank.
        let c = encode("int64", &["", ""]);
        assert!(c.is_err());
    }

    #[test]
    fn malformed_cells_are_reported_with_their_row() {
        let bad = encode("int64", &["1", "x"]).unwrap_err().to_string();
        assert!(
            bad.contains("whole number") && bad.contains("row 3"),
            "{bad}"
        );
        assert!(encode("double", &["inf"]).is_err());
        assert!(encode("dateTime", &["2020-02-30x"]).is_err());
    }

    #[test]
    fn a_table_builds_its_catalog_and_column_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.csv");
        std::fs::write(&path, "id,name,amount\n1,Ann,2.5\n2,Bo,10\n3,Ann,7\n").unwrap();
        let table = ModelTable {
            name: "people".into(),
            columns: vec![
                column("id", "int64"),
                column("name", "string"),
                column("amount", "double"),
            ],
            csv_path: Some(path.to_string_lossy().into_owned()),
            ..ModelTable::default()
        };
        let model = Model {
            tables: vec![table],
            relationships: Vec::new(),
        };
        let built = build(
            &model,
            &[("people".into(), vec!["let".into()])],
            &HashMap::new(),
        )
        .unwrap();
        assert!(built.catalog.starts_with(b"SQLite format 3"));
        let names: Vec<&str> = built.files.iter().map(|(p, _)| p.as_str()).collect();
        // The row-number column and three data columns: an idf and idfmeta each.
        assert_eq!(names.iter().filter(|n| n.ends_with(".idf")).count(), 4);
        assert_eq!(names.iter().filter(|n| n.ends_with(".idfmeta")).count(), 4);
        // Dictionaries for the three data columns.
        assert_eq!(
            names.iter().filter(|n| n.ends_with(".dictionary")).count(),
            3
        );
        assert!(names.iter().all(|n| n.starts_with("people (")), "{names:?}");
    }
}

#[cfg(test)]
mod calculated_tests {
    use super::*;

    #[test]
    fn a_calculated_column_takes_its_type_from_its_values() {
        assert_eq!(infer_type(&["1", "2", "3"]), "int64");
        assert_eq!(infer_type(&["1", "2.5"]), "double");
        assert_eq!(infer_type(&["2020-01-02", "2021-03-04"]), "dateTime");
        assert_eq!(infer_type(&["true", "FALSE"]), "boolean");
        assert_eq!(infer_type(&["big", "small"]), "string");
        assert_eq!(infer_type(&["1", "x"]), "string");
        assert_eq!(infer_type(&[]), "string");
    }

    fn people(dir: &std::path::Path) -> ModelTable {
        let path = dir.join("t.csv");
        std::fs::write(&path, "id,amount\n1,2.5\n2,10\n3,7\n").unwrap();
        let column = |name: &str, kind: &str, expression: Option<&str>| ModelColumn {
            name: name.into(),
            data_type: kind.into(),
            expression: expression.map(String::from),
        };
        ModelTable {
            name: "people".into(),
            columns: vec![
                column("id", "int64", None),
                column("amount", "double", None),
                column(
                    "tier",
                    "double",
                    Some("IF([amount] > 5, \"high\", \"low\")"),
                ),
            ],
            csv_path: Some(path.to_string_lossy().into_owned()),
            ..ModelTable::default()
        }
    }

    #[test]
    fn calculated_columns_are_stored_with_their_computed_values() {
        let dir = tempfile::tempdir().unwrap();
        let table = people(dir.path());
        let mut values = HashMap::new();
        values.insert(
            ("people".to_string(), "tier".to_string()),
            vec!["low".to_string(), "high".to_string(), "high".to_string()],
        );
        let data = load_table(&table, &values).unwrap();
        let tier = &data.columns[2];
        // The declared type ("double") is replaced by what the values are.
        assert_eq!(tier.data_type, "string");
        assert!(tier.expression.as_deref().unwrap().starts_with("IF("));
        assert!(matches!(&tier.encoding, Encoding::Strings(v) if v == &["low", "high"]));
        assert!(data.columns[0].expression.is_none());

        let model = Model {
            tables: vec![table],
            relationships: Vec::new(),
        };
        let built = build(&model, &[("people".into(), vec!["let".into()])], &values).unwrap();
        // Row number plus three columns, each with an idf and an idfmeta.
        let idfs = built
            .files
            .iter()
            .filter(|(p, _)| p.ends_with(".idf"))
            .count();
        assert_eq!(idfs, 4);
    }

    #[test]
    fn a_calculated_column_without_computed_values_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = load_table(&people(dir.path()), &HashMap::new())
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("no values were computed"), "{err}");
    }

    #[test]
    fn computed_values_must_match_the_row_count() {
        let dir = tempfile::tempdir().unwrap();
        let table = people(dir.path());
        let mut values = HashMap::new();
        values.insert(
            ("people".to_string(), "tier".to_string()),
            vec!["low".to_string()],
        );
        let err = load_table(&table, &values).err().unwrap().to_string();
        assert!(err.contains("1 values for 3 rows"), "{err}");
    }
}
