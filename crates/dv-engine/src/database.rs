//! Database source support: connection references and table discovery.
//!
//! A `.dvz` report names a database only by a connection reference. The
//! reference `<name>` is resolved from the environment variable
//! `DAVILIZE_CONN_<NAME>` (name upper-cased, `-` replaced by `_`), so that
//! connection strings, passwords and file paths never enter a report.

use duckdb::Connection;

use crate::EngineError;

const CONNECTION_ENV_PREFIX: &str = "DAVILIZE_CONN_";

/// The environment variable that holds the target of a connection reference.
///
/// Only letters, digits, `_` and `-` are accepted, so the name cannot inject
/// anything into the variable lookup or into SQL.
pub fn connection_env_var(connection: &str) -> Result<String, EngineError> {
    let valid = !connection.is_empty()
        && connection
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !valid {
        return Err(EngineError::InvalidSpec(format!(
            "connection reference '{connection}' must use only letters, digits, '_' or '-'"
        )));
    }
    let normalized = connection.replace('-', "_").to_ascii_uppercase();
    Ok(format!("{CONNECTION_ENV_PREFIX}{normalized}"))
}

/// Resolves a connection reference to its target: a SQLite file path or a
/// Postgres connection string, read from `DAVILIZE_CONN_<NAME>`.
pub fn resolve_connection(connection: &str) -> Result<String, EngineError> {
    let env_var = connection_env_var(connection)?;
    match std::env::var(&env_var) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(EngineError::MissingConnection {
            connection: connection.to_string(),
            env_var,
        }),
    }
}

/// Replaces every occurrence of `secret` in `message`, so that connection
/// strings with passwords do not appear in errors.
pub(crate) fn redact(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        message.to_string()
    } else {
        message.replace(secret, "<redacted>")
    }
}

/// Lists `(schema, table)` pairs of an attached catalog, skipping system schemas.
pub(crate) fn list_tables(
    conn: &Connection,
    catalog: &str,
) -> Result<Vec<(String, String)>, EngineError> {
    let mut stmt = conn.prepare(
        "SELECT table_schema, table_name FROM information_schema.tables \
         WHERE table_catalog = ? AND table_schema NOT IN ('information_schema', 'pg_catalog') \
         ORDER BY table_schema, table_name",
    )?;
    let rows = stmt.query_map([catalog], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut tables = Vec::new();
    for row in rows {
        tables.push(row?);
    }
    Ok(tables)
}
