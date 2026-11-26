//! Read-only access to SQLite files, for database sources in the browser.
//!
//! DuckDB-Wasm's `sqlite` extension cannot open files the page registered (it uses SQLite's own
//! file layer), so the page hands the file's bytes to this module instead. SQLite itself is the
//! WebAssembly build that rusqlite uses (`sqlite-wasm-rs`); each table or view is read into Arrow
//! and the page inserts it into DuckDB as a table, with a view per table in a schema named after
//! the source, as the desktop engine does.
//!
//! Column types follow the declared type, SQLite-style (`INT` is an integer, `REAL`, `FLOA` and
//! `DOUB` are doubles, `CHAR`, `CLOB` and `TEXT` are text, `BLOB` is binary), plus `DATE`,
//! `DATETIME`/`TIMESTAMP` and `BOOL`. A column with no declared type takes the type its values
//! share. A value that does not fit the declared type turns the whole column into text, so no
//! value is ever lost or changed.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Cursor;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float64Builder, Int64Builder,
    StringBuilder, TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::ipc::writer::StreamWriter;
use arrow::record_batch::RecordBatch;
use chrono::{NaiveDate, NaiveDateTime};
use rusqlite::types::Value;
use rusqlite::{Connection, MAIN_DB};

thread_local! {
    static DATABASES: RefCell<BTreeMap<String, Connection>> = const { RefCell::new(BTreeMap::new()) };
}

/// Opens a SQLite file from its bytes (read only) and keeps it under `name`. Returns the names
/// of its tables and views, without SQLite's internal ones.
pub fn open(name: &str, bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
    conn.deserialize_read_exact(MAIN_DB, Cursor::new(bytes), bytes.len(), true)
        .map_err(|e| format!("not a readable SQLite database: {e}"))?;
    let tables = {
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') \
                 AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' ORDER BY name",
            )
            .map_err(|e| format!("not a readable SQLite database: {e}"))?;
        stmt.query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
            .map_err(|e| format!("not a readable SQLite database: {e}"))?
    };
    DATABASES.with(|d| d.borrow_mut().insert(name.to_string(), conn));
    Ok(tables)
}

/// Forgets a database opened with [`open`].
pub fn close(name: &str) {
    DATABASES.with(|d| d.borrow_mut().remove(name));
}

/// One table or view as an Arrow IPC stream.
pub fn table_ipc(name: &str, table: &str) -> Result<Vec<u8>, String> {
    DATABASES.with(|databases| {
        let databases = databases.borrow();
        let conn = databases
            .get(name)
            .ok_or_else(|| format!("database '{name}' is not open"))?;
        let batch = read_table(conn, table)?;
        let mut writer = StreamWriter::try_new(Vec::new(), &batch.schema())
            .map_err(|e| format!("cannot write '{table}': {e}"))?;
        writer
            .write(&batch)
            .and_then(|()| writer.finish())
            .map_err(|e| format!("cannot write '{table}': {e}"))?;
        writer
            .into_inner()
            .map_err(|e| format!("cannot write '{table}': {e}"))
    })
}

fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn read_table(conn: &Connection, table: &str) -> Result<RecordBatch, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT * FROM {}", quote(table)))
        .map_err(|e| format!("cannot read '{table}': {e}"))?;
    let names: Vec<String> = stmt.column_names().map(str::to_string).collect();
    let declared: Vec<Option<String>> = stmt
        .columns()
        .map(|c| c.decl_type().map(str::to_ascii_uppercase))
        .collect();
    let mut columns: Vec<Vec<Value>> = vec![Vec::new(); names.len()];
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("cannot read '{table}': {e}"))?;
    while let Some(row) = rows
        .next()
        .map_err(|e| format!("cannot read '{table}': {e}"))?
    {
        for (index, column) in columns.iter_mut().enumerate() {
            column.push(
                row.get::<_, Value>(index)
                    .map_err(|e| format!("cannot read '{table}': {e}"))?,
            );
        }
    }
    let mut fields = Vec::with_capacity(names.len());
    let mut arrays: Vec<ArrayRef> = Vec::with_capacity(names.len());
    for ((name, decl), values) in names.iter().zip(&declared).zip(&columns) {
        let (data_type, array) = build_column(decl.as_deref(), values);
        fields.push(Field::new(name, data_type, true));
        arrays.push(array);
    }
    RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays)
        .map_err(|e| format!("cannot read '{table}': {e}"))
}

/// What a declared type asks for, before the values are checked against it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Want {
    Integer,
    Real,
    Text,
    Blob,
    Bool,
    Date,
    Timestamp,
    /// No declared type: take whatever the values share.
    Infer,
}

fn want(declared: Option<&str>) -> Want {
    let Some(decl) = declared else {
        return Want::Infer;
    };
    // SQLite's own affinity rules, in its order, with a few extra names first.
    if decl.contains("BOOL") {
        Want::Bool
    } else if decl.contains("DATETIME") || decl.contains("TIMESTAMP") {
        Want::Timestamp
    } else if decl.contains("DATE") {
        Want::Date
    } else if decl.contains("INT") {
        Want::Integer
    } else if decl.contains("CHAR") || decl.contains("CLOB") || decl.contains("TEXT") {
        Want::Text
    } else if decl.contains("BLOB") || decl.is_empty() {
        if decl.is_empty() {
            Want::Infer
        } else {
            Want::Blob
        }
    } else if decl.contains("REAL")
        || decl.contains("FLOA")
        || decl.contains("DOUB")
        || decl.contains("NUMERIC")
        || decl.contains("DECIMAL")
    {
        Want::Real
    } else {
        Want::Infer
    }
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(text.trim().get(..10)?, "%Y-%m-%d").ok()
}

fn parse_timestamp(text: &str) -> Option<NaiveDateTime> {
    let text = text.trim();
    [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%dT%H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(text, format).ok())
    .or_else(|| parse_date(text).and_then(|d| d.and_hms_opt(0, 0, 0)))
}

fn epoch_days(date: NaiveDate) -> i32 {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1).expect("valid date");
    i32::try_from((date - epoch).num_days()).unwrap_or(0)
}

fn fits(want: Want, values: &[Value]) -> bool {
    values.iter().all(|v| match (want, v) {
        (_, Value::Null) => true,
        (Want::Integer, Value::Integer(_)) => true,
        (Want::Real, Value::Integer(_) | Value::Real(_)) => true,
        (Want::Text, Value::Text(_)) => true,
        (Want::Blob, Value::Blob(_)) => true,
        (Want::Bool, Value::Integer(0 | 1)) => true,
        (Want::Date, Value::Text(t)) => parse_date(t).is_some(),
        (Want::Timestamp, Value::Text(t)) => parse_timestamp(t).is_some(),
        _ => false,
    })
}

/// The type shared by every non-null value of an undeclared column.
fn infer(values: &[Value]) -> Want {
    let mut seen_int = false;
    let mut seen_real = false;
    let mut seen_text = false;
    let mut seen_blob = false;
    for v in values {
        match v {
            Value::Null => {}
            Value::Integer(_) => seen_int = true,
            Value::Real(_) => seen_real = true,
            Value::Text(_) => seen_text = true,
            Value::Blob(_) => seen_blob = true,
        }
    }
    if seen_text || (seen_blob && (seen_int || seen_real)) {
        Want::Text
    } else if seen_blob {
        Want::Blob
    } else if seen_real {
        Want::Real
    } else if seen_int {
        Want::Integer
    } else {
        Want::Text
    }
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::Integer(i) => Some(i.to_string()),
        Value::Real(r) => Some(r.to_string()),
        Value::Text(t) => Some(t.clone()),
        Value::Blob(b) => Some(b.iter().map(|byte| format!("{byte:02x}")).collect()),
    }
}

fn build_column(declared: Option<&str>, values: &[Value]) -> (DataType, ArrayRef) {
    let mut kind = want(declared);
    if kind == Want::Infer {
        kind = infer(values);
    }
    if !fits(kind, values) {
        kind = Want::Text;
    }
    match kind {
        Want::Integer => {
            let mut b = Int64Builder::new();
            for v in values {
                match v {
                    Value::Integer(i) => b.append_value(*i),
                    _ => b.append_null(),
                }
            }
            (DataType::Int64, Arc::new(b.finish()))
        }
        Want::Real => {
            let mut b = Float64Builder::new();
            for v in values {
                match v {
                    Value::Integer(i) => b.append_value(*i as f64),
                    Value::Real(r) => b.append_value(*r),
                    _ => b.append_null(),
                }
            }
            (DataType::Float64, Arc::new(b.finish()))
        }
        Want::Blob => {
            let mut b = BinaryBuilder::new();
            for v in values {
                match v {
                    Value::Blob(bytes) => b.append_value(bytes),
                    _ => b.append_null(),
                }
            }
            (DataType::Binary, Arc::new(b.finish()))
        }
        Want::Bool => {
            let mut b = BooleanBuilder::new();
            for v in values {
                match v {
                    Value::Integer(i) => b.append_value(*i == 1),
                    _ => b.append_null(),
                }
            }
            (DataType::Boolean, Arc::new(b.finish()))
        }
        Want::Date => {
            let mut b = Date32Builder::new();
            for v in values {
                match v {
                    Value::Text(t) => b.append_value(epoch_days(parse_date(t).expect("checked"))),
                    _ => b.append_null(),
                }
            }
            (DataType::Date32, Arc::new(b.finish()))
        }
        Want::Timestamp => {
            let mut b = TimestampMicrosecondBuilder::new();
            for v in values {
                match v {
                    Value::Text(t) => b.append_value(
                        parse_timestamp(t)
                            .expect("checked")
                            .and_utc()
                            .timestamp_micros(),
                    ),
                    _ => b.append_null(),
                }
            }
            (
                DataType::Timestamp(TimeUnit::Microsecond, None),
                Arc::new(b.finish()),
            )
        }
        Want::Text | Want::Infer => {
            let mut b = StringBuilder::new();
            for v in values {
                match as_text(v) {
                    Some(text) => b.append_value(text),
                    None => b.append_null(),
                }
            }
            (DataType::Utf8, Arc::new(b.finish()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::{Array, AsArray};
    use arrow::datatypes::{Float64Type, Int64Type};
    use arrow::ipc::reader::StreamReader;

    /// A small SQLite database, as the bytes of a file.
    fn sample() -> Vec<u8> {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, region TEXT, amount REAL, placed DATE, \
                                  paid BOOLEAN, note);
             INSERT INTO orders VALUES (1, 'North', 12.5, '2026-01-03', 1, 'a');
             INSERT INTO orders VALUES (2, 'South', 7, '2026-01-05', 0, NULL);
             INSERT INTO orders VALUES (3, NULL, NULL, NULL, NULL, 'c');
             CREATE TABLE mixed (n INTEGER);
             INSERT INTO mixed VALUES (1);
             INSERT INTO mixed VALUES ('two');
             CREATE VIEW north AS SELECT * FROM orders WHERE region = 'North';
             CREATE TABLE empty_one (x INT, y TEXT);",
        )
        .unwrap();
        conn.serialize(MAIN_DB).unwrap().to_vec()
    }

    fn read(ipc: &[u8]) -> RecordBatch {
        let mut reader = StreamReader::try_new(Cursor::new(ipc.to_vec()), None).unwrap();
        reader.next().unwrap().unwrap()
    }

    #[test]
    fn lists_tables_and_views_without_internal_ones() {
        let tables = open("t1", &sample()).unwrap();
        assert_eq!(tables, ["empty_one", "mixed", "north", "orders"]);
        close("t1");
    }

    #[test]
    fn declared_types_become_arrow_types() {
        open("t2", &sample()).unwrap();
        let batch = read(&table_ipc("t2", "orders").unwrap());
        assert_eq!(batch.num_rows(), 3);
        let schema = batch.schema();
        let types: Vec<&DataType> = schema.fields().iter().map(|f| f.data_type()).collect();
        assert_eq!(
            types.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            ["Int64", "Utf8", "Float64", "Date32", "Boolean", "Utf8"]
        );
        assert_eq!(batch.column(0).as_primitive::<Int64Type>().value(1), 2);
        // An integer stored in a REAL column reads as a double.
        assert_eq!(batch.column(2).as_primitive::<Float64Type>().value(1), 7.0);
        assert!(batch.column(2).is_null(2));
        assert!(batch.column(1).is_null(2));
        close("t2");
    }

    #[test]
    fn a_value_that_does_not_fit_turns_the_column_into_text() {
        open("t3", &sample()).unwrap();
        let batch = read(&table_ipc("t3", "mixed").unwrap());
        assert_eq!(batch.schema().field(0).data_type(), &DataType::Utf8);
        let text = batch.column(0).as_string::<i32>();
        assert_eq!((text.value(0), text.value(1)), ("1", "two"));
        close("t3");
    }

    #[test]
    fn views_and_empty_tables_are_readable() {
        open("t4", &sample()).unwrap();
        assert_eq!(read(&table_ipc("t4", "north").unwrap()).num_rows(), 1);
        assert_eq!(read(&table_ipc("t4", "empty_one").unwrap()).num_rows(), 0);
        close("t4");
    }

    #[test]
    fn a_missing_database_or_a_bad_file_is_an_error() {
        assert!(table_ipc("never_opened", "x").is_err());
        let err = open("bad", b"this is not a database").unwrap_err();
        assert!(err.contains("not a readable SQLite database"), "{err}");
    }

    #[test]
    fn declared_types_follow_sqlite_affinity() {
        assert_eq!(want(Some("BIGINT")), Want::Integer);
        assert_eq!(want(Some("VARCHAR(20)")), Want::Text);
        assert_eq!(want(Some("DOUBLE PRECISION")), Want::Real);
        assert_eq!(want(Some("DATETIME")), Want::Timestamp);
        assert_eq!(want(Some("DECIMAL(10,2)")), Want::Real);
        assert_eq!(want(None), Want::Infer);
    }
}
