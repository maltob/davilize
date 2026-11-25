//! Validation of calculated-field expressions on a DuckDB connection. The allowlist and
//! the tree walk live in `dv_sql::calc` (they need no database, so the browser build
//! shares them); this module only asks DuckDB to parse the expression.

use duckdb::Connection;

use crate::EngineError;

/// Checks one calculated field's expression. `field` names it in error messages.
pub fn check(conn: &Connection, field: &str, expression: &str) -> Result<(), EngineError> {
    dv_sql::calc::precheck(field, expression)?;
    let text: String =
        conn.query_row(&dv_sql::calc::parse_query(expression), [], |row| row.get(0))?;
    Ok(dv_sql::calc::verify(field, &text)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dv_sql::calc::{ALLOWED_FUNCTIONS, MAX_EXPRESSION_CHARS};

    fn verdict(expression: &str) -> Result<(), String> {
        let conn = Connection::open_in_memory().unwrap();
        check(&conn, "f", expression).map_err(|e| e.to_string())
    }

    #[test]
    fn allows_columns_constants_operators_and_allowed_functions() {
        for expression in [
            r#""revenue" - "cost""#,
            "units * 2 + 1",
            "round(revenue / units, 2)",
            "CASE WHEN units > 3 THEN 'big' ELSE 'small' END",
            "CAST(units AS DOUBLE) / 3",
            "units::VARCHAR",
            "DATE '2024-01-01' + 1",
            "CAST(units AS DECIMAL(10, 2))",
            "coalesce(discount, 0)",
            "nullif(region, 'East')",
            "upper(region) || '-' || product",
            "units BETWEEN 1 AND 5",
            "region IN ('North', 'South')",
            "region NOT IN ('East')",
            "NOT (units > 3) AND region IS NOT NULL",
            "date_part('year', order_date)",
            "EXTRACT(year FROM order_date)",
            "order_date + 7",
            "region LIKE 'N%'",
            "if(units > 3, 'a', 'b')",
            "-units",
            "1.5e3",
            "x.y",
        ] {
            assert!(
                verdict(expression).is_ok(),
                "should allow {expression}: {:?}",
                verdict(expression)
            );
        }
    }

    #[test]
    fn rejects_table_and_file_functions() {
        for expression in [
            "read_csv('/etc/passwd')",
            "read_csv_auto('x.csv')",
            "read_parquet('x.parquet')",
            "read_text('x')",
            "glob('*')",
            "getenv('HOME')",
            "current_setting('home_directory')",
            "sniff_csv('x')",
            "query('SELECT 1')",
            "count(*)",
            "sum(units)",
            "list(units)",
            "row_number() OVER ()",
            "repeat('x', 1000000000)",
            "lpad('x', 1000000000, ' ')",
            "now()",
            "random()",
            "main.upper(region)",
        ] {
            let err = verdict(expression).expect_err(expression);
            assert!(err.contains("calculated field 'f'"), "{expression}: {err}");
        }
    }

    #[test]
    fn rejects_subqueries_and_statements() {
        for expression in [
            "(SELECT 1)",
            "units IN (SELECT units FROM sales)",
            "EXISTS (SELECT 1)",
            "(SELECT max(units) FROM sales)",
            "1 FROM read_csv('x')",
            "1) FROM read_csv('x') --",
            "1; DROP TABLE sales",
            "1); DROP TABLE sales; SELECT (1",
            "1 UNION SELECT 2",
            "1 AS x",
            "*",
            "?",
            "$1",
            "attach 'x.db'",
            "copy sales to 'out.csv'",
            "units WHERE 1 = 1",
        ] {
            assert!(verdict(expression).is_err(), "should reject {expression}");
        }
    }

    #[test]
    fn rejects_comment_tricks_and_control_characters() {
        for expression in [
            "1 -- comment",
            "1 /* hidden */ + 2",
            "1 */ + 2",
            "'a;b'",
            "units\0",
            "units\u{7}",
        ] {
            assert!(verdict(expression).is_err(), "should reject {expression:?}");
        }
    }

    #[test]
    fn rejects_empty_and_overlong_expressions() {
        assert!(verdict("   ").is_err());
        let long = "1 + ".repeat(MAX_EXPRESSION_CHARS);
        let err = verdict(&long).unwrap_err();
        assert!(err.contains("longer than"), "{err}");
    }

    #[test]
    fn reports_syntax_errors_with_the_field_name() {
        let err = verdict("units +").unwrap_err();
        assert!(err.contains("invalid expression"), "{err}");
    }

    #[test]
    fn allowlists_do_not_overlap_with_forbidden_names() {
        for forbidden in [
            "read_csv",
            "glob",
            "getenv",
            "current_setting",
            "repeat",
            "lpad",
            "rpad",
            "now",
            "random",
        ] {
            assert!(!ALLOWED_FUNCTIONS.contains(&forbidden), "{forbidden}");
        }
    }
}
