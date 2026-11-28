//! Guessing better column types from a sample of values.
//!
//! DuckDB already types most CSV columns. What it leaves as text: numbers written with a
//! currency symbol, thousands separators, a percent sign or accounting brackets; numbers mixed
//! with "n/a"-style markers; and dates written with month names ("Jan 5, 2025"). This looks at
//! the first [`SAMPLE_ROWS`] values of each text column and, when every value fits, returns a
//! SQL expression that reads the column as a number or a date. Text such as zip codes with
//! leading zeros is left alone.
//!
//! The sample only proposes a conversion. Before it is used, [`verification_sql`] counts the
//! values anywhere in the column that would not read (they would become NULL, or lose a leading
//! zero), and [`keep_readable`] drops every conversion with even one. A column is converted only
//! when all of it fits, so no value is ever lost.

use crate::{quote_ident, quote_literal};

/// How many rows of a text column are looked at.
pub const SAMPLE_ROWS: usize = 100;

/// A column needs at least this many real values (not blank, not "n/a") to be converted.
const MIN_VALUES: usize = 2;

/// What a text column is read as instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Integer,
    Number,
    Date,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Integer => "integer",
            Target::Number => "number",
            Target::Date => "date",
        }
    }
}

/// A text column that should be read as something else, with the SQL expression that does it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub column: String,
    pub target: Target,
    /// An expression over the column, for use in `SELECT * REPLACE (<expr> AS "<column>")`.
    pub expr: String,
    /// A SQL condition over the column, true for a value that this conversion would not read
    /// faithfully (see [`verification_sql`]).
    pub unreadable: String,
}

impl Conversion {
    /// `<expr> AS "<column>"`, one entry of a `* REPLACE (...)` clause.
    pub fn replace_part(&self) -> String {
        format!("{} AS {}", self.expr, quote_ident(&self.column))
    }
}

/// Markers that mean "no value" and are read as NULL when a number column is converted.
const MISSING: [&str; 9] = ["", "n/a", "na", "null", "none", "nan", "-", "--", "?"];

fn is_missing(value: &str) -> bool {
    MISSING.contains(&value.to_ascii_lowercase().as_str())
}

/// SQL that is true for a value that is present and not a missing marker.
fn present_sql(column: &str) -> String {
    let list: Vec<String> = MISSING.iter().map(|m| quote_literal(m)).collect();
    let c = quote_ident(column);
    format!(
        "({c} IS NOT NULL AND lower(trim(CAST({c} AS VARCHAR))) NOT IN ({}))",
        list.join(", ")
    )
}

struct Num {
    /// Has a currency symbol, thousands separators, a percent sign or brackets.
    decorated: bool,
    decimal: bool,
}

/// Reads `$1,234.50`, `12%`, `(3,000)`, `-7`, `1.5`. Plain whole numbers with a leading zero
/// (zip codes, product codes) are not numbers.
fn parse_number(raw: &str) -> Option<Num> {
    let mut s = raw.trim();
    let mut decorated = false;
    if s.len() > 2 && s.starts_with('(') && s.ends_with(')') {
        s = s[1..s.len() - 1].trim();
        decorated = true;
    }
    if let Some(rest) = s.strip_prefix(|c| c == '-' || c == '+') {
        s = rest.trim_start();
    }
    if let Some(rest) = s.strip_prefix(|c| matches!(c, '$' | '€' | '£' | '¥')) {
        s = rest.trim_start();
        decorated = true;
    }
    if let Some(rest) = s.strip_suffix('%') {
        s = rest.trim_end();
        decorated = true;
    }
    let (whole, fraction) = match s.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (s, None),
    };
    if whole.is_empty() && fraction.is_none() {
        return None;
    }
    if let Some(fraction) = fraction
        && (fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    if whole.contains(',') {
        let groups: Vec<&str> = whole.split(',').collect();
        let digits = groups.iter().all(|g| g.bytes().all(|b| b.is_ascii_digit()));
        let shaped = !groups[0].is_empty()
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3);
        if !digits || !shaped {
            return None;
        }
        decorated = true;
    } else {
        if !whole.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        if whole.len() > 1 && whole.starts_with('0') {
            return None;
        }
    }
    Some(Num {
        decorated,
        decimal: fraction.is_some(),
    })
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];

/// Date layouts with a month name, so day and month can never be swapped. They are written as
/// DuckDB `strptime` formats, and [`fits_format`] checks the same layouts.
const DATE_FORMATS: &[&str] = &[
    "%b %d, %Y",
    "%b %d %Y",
    "%d %b %Y",
    "%d-%b-%Y",
    "%d-%b-%y",
    "%d/%b/%Y",
    "%B %d, %Y",
    "%B %d %Y",
    "%d %B %Y",
];

/// Whether `value` is written in exactly this layout.
fn fits_format(value: &str, format: &str) -> bool {
    let mut rest = value;
    let mut spec = format.chars().peekable();
    while let Some(c) = spec.next() {
        if c != '%' {
            match rest.strip_prefix(c) {
                Some(after) => rest = after,
                None => return false,
            }
            continue;
        }
        let Some(kind) = spec.next() else {
            return false;
        };
        match kind {
            'b' | 'B' => {
                let end = rest
                    .find(|ch: char| !ch.is_ascii_alphabetic())
                    .unwrap_or(rest.len());
                let word = rest[..end].to_ascii_lowercase();
                let known = MONTHS.iter().any(|m| {
                    if kind == 'B' {
                        *m == word
                    } else {
                        word.len() == 3 && m.starts_with(word.as_str())
                    }
                });
                if !known {
                    return false;
                }
                rest = &rest[end..];
            }
            'd' => {
                let end = rest.bytes().take(2).take_while(u8::is_ascii_digit).count();
                match rest[..end].parse::<u32>() {
                    Ok(day) if (1..=31).contains(&day) => rest = &rest[end..],
                    _ => return false,
                }
            }
            'Y' | 'y' => {
                let width = if kind == 'Y' { 4 } else { 2 };
                let end = rest.bytes().take_while(u8::is_ascii_digit).count();
                if end != width {
                    return false;
                }
                rest = &rest[end..];
            }
            _ => return false,
        }
    }
    rest.is_empty()
}

/// The conversion for one text column, from up to [`SAMPLE_ROWS`] of its values (`None` is NULL).
pub fn refine(column: &str, values: &[Option<String>]) -> Option<Conversion> {
    let present: Vec<&str> = values.iter().flatten().map(|v| v.trim()).collect();
    let real: Vec<&str> = present.iter().copied().filter(|v| !is_missing(v)).collect();
    // A marker such as "n/a" is probably why the column was read as text.
    let marker = present.iter().any(|v| !v.is_empty() && is_missing(v));
    if real.len() < MIN_VALUES {
        return None;
    }
    as_number(column, &real, marker).or_else(|| as_date(column, &real))
}

fn as_number(column: &str, real: &[&str], marker: bool) -> Option<Conversion> {
    let parsed: Vec<Num> = real
        .iter()
        .map(|v| parse_number(v))
        .collect::<Option<_>>()?;
    // Plain numbers with nothing else would have been read as numbers already, so a text column
    // of them is text on purpose (or has a value later in the file that is not a number).
    if !marker && !parsed.iter().any(|n| n.decorated) {
        return None;
    }
    let target = if parsed.iter().any(|n| n.decimal) {
        Target::Number
    } else {
        Target::Integer
    };
    let sql_type = if target == Target::Number {
        "DOUBLE"
    } else {
        "BIGINT"
    };
    let c = quote_ident(column);
    // Brackets mean negative; then the symbols, separators and spaces go; what cannot be read
    // (an "n/a") becomes NULL.
    let expr = format!(
        r"TRY_CAST(regexp_replace(regexp_replace(trim(CAST({c} AS VARCHAR)), '^\((.*)\)$', '-\1'), '[$€£¥,%\s]', '', 'g') AS {sql_type})"
    );
    // A value that does not read becomes NULL, and a code with a leading zero would lose it.
    let unreadable = format!(
        r"({} AND ({expr} IS NULL OR regexp_matches(trim(CAST({c} AS VARCHAR)), '^\(?[-+]?[$€£¥]?\s*0[0-9]')))",
        present_sql(column)
    );
    Some(Conversion {
        column: column.to_string(),
        target,
        expr,
        unreadable,
    })
}

fn as_date(column: &str, real: &[&str]) -> Option<Conversion> {
    let format = DATE_FORMATS
        .iter()
        .copied()
        .find(|f| real.iter().all(|v| fits_format(v, f)))?;
    let c = quote_ident(column);
    let expr = format!(
        "CAST(try_strptime(trim(CAST({c} AS VARCHAR)), {}) AS DATE)",
        quote_literal(format)
    );
    let unreadable = format!("({} AND {expr} IS NULL)", present_sql(column));
    Some(Conversion {
        column: column.to_string(),
        target: Target::Date,
        expr,
        unreadable,
    })
}

/// Conversions for every sampled text column that needs one.
pub fn refine_columns(samples: &[(String, Vec<Option<String>>)]) -> Vec<Conversion> {
    samples
        .iter()
        .filter_map(|(column, values)| refine(column, values))
        .collect()
}

/// `* REPLACE (<expr> AS "<column>", ...)` for the conversions, or None when there are none.
pub fn replace_clause(conversions: &[Conversion]) -> Option<String> {
    if conversions.is_empty() {
        return None;
    }
    let parts: Vec<String> = conversions.iter().map(Conversion::replace_part).collect();
    Some(format!("* REPLACE ({})", parts.join(", ")))
}

/// A query over `view` (the text columns as read) giving, per conversion and in order, how many
/// values in the whole column would not read. `None` when there is nothing to check.
pub fn verification_sql(view: &str, conversions: &[Conversion]) -> Option<String> {
    if conversions.is_empty() {
        return None;
    }
    let counts: Vec<String> = conversions
        .iter()
        .enumerate()
        .map(|(i, c)| format!("count(*) FILTER (WHERE {}) AS \"c{i}\"", c.unreadable))
        .collect();
    Some(format!("SELECT {} FROM {view}", counts.join(", ")))
}

/// The conversions whose column has no unreadable value; `failures` is the answer to
/// [`verification_sql`].
pub fn keep_readable(conversions: Vec<Conversion>, failures: &[i64]) -> Vec<Conversion> {
    conversions
        .into_iter()
        .zip(failures)
        .filter(|(_, bad)| **bad == 0)
        .map(|(c, _)| c)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(items: &[&str]) -> Vec<Option<String>> {
        items.iter().map(|s| Some(s.to_string())).collect()
    }

    fn target(items: &[&str]) -> Option<Target> {
        refine("c", &values(items)).map(|c| c.target)
    }

    #[test]
    fn currency_and_thousands_separators_are_numbers() {
        assert_eq!(
            target(&["$1,234.50", "$12.00", "$0.99"]),
            Some(Target::Number)
        );
        assert_eq!(target(&["1,200", "35,000", "7"]), Some(Target::Integer));
        assert_eq!(target(&["€5", "€10"]), Some(Target::Integer));
    }

    #[test]
    fn percentages_and_brackets_are_numbers() {
        assert_eq!(target(&["12%", "7.5%", "100%"]), Some(Target::Number));
        assert_eq!(target(&["(1,500)", "2,000", "(30)"]), Some(Target::Integer));
        assert_eq!(target(&["-$5.00", "$3.25"]), Some(Target::Number));
    }

    #[test]
    fn missing_markers_do_not_stop_a_number_column() {
        assert_eq!(
            target(&["12", "n/a", "30", "-", "41"]),
            Some(Target::Integer)
        );
        assert_eq!(target(&["1.5", "N/A", "2.25", ""]), Some(Target::Number));
    }

    #[test]
    fn plain_numbers_in_a_text_column_are_left_alone() {
        // Nothing marks them as numbers written in a human way, so the column is text on purpose.
        assert_eq!(target(&["12", "30", "41"]), None);
        assert_eq!(target(&["1.5", "2.25"]), None);
    }

    #[test]
    fn codes_with_leading_zeros_stay_text() {
        assert_eq!(target(&["02134", "10001", "90210"]), None);
        assert_eq!(target(&["007", "n/a", "042"]), None);
        // A lone zero or 0.5 is a number.
        assert_eq!(target(&["0", "n/a", "12"]), Some(Target::Integer));
        assert_eq!(target(&["0.5", "n/a"]), None); // only one real value
    }

    #[test]
    fn one_value_that_does_not_fit_keeps_the_column_text() {
        assert_eq!(target(&["$5", "$6", "see note"]), None);
        assert_eq!(target(&["1,23", "$5"]), None);
        assert_eq!(target(&["1,2345", "$5"]), None);
    }

    #[test]
    fn dates_with_month_names_are_dates() {
        assert_eq!(
            target(&["Jan 5, 2025", "Mar 12, 2024", "Dec 31, 2023"]),
            Some(Target::Date)
        );
        assert_eq!(target(&["5 Jan 2025", "12 March 2024"]), None); // %b and %B are not mixed
        assert_eq!(target(&["5 Jan 2025", "12 Mar 2024"]), Some(Target::Date));
        assert_eq!(target(&["05-Jan-25", "12-Mar-24"]), Some(Target::Date));
        assert_eq!(
            target(&["January 5, 2025", "March 12, 2024"]),
            Some(Target::Date)
        );
        assert_eq!(target(&["jan 5 2025", "FEB 6 2025"]), Some(Target::Date));
    }

    #[test]
    fn impossible_dates_and_other_text_stay_text() {
        assert_eq!(target(&["Jan 32, 2025", "Mar 12, 2024"]), None);
        assert_eq!(target(&["Foo 5, 2025", "Mar 12, 2024"]), None);
        assert_eq!(target(&["Jan 5, 25", "Mar 12, 24"]), None);
        assert_eq!(target(&["red", "green", "blue"]), None);
    }

    #[test]
    fn too_few_values_are_not_enough() {
        assert_eq!(target(&["$5"]), None);
        assert_eq!(target(&["$5", "n/a"]), None);
        assert!(refine("c", &[None, None]).is_none());
    }

    #[test]
    fn the_replace_clause_names_each_column() {
        let samples = vec![
            ("price".to_string(), values(&["$1", "$2.5"])),
            ("note".to_string(), values(&["a", "b"])),
            ("when".to_string(), values(&["Jan 5, 2025", "Feb 6, 2025"])),
        ];
        let conversions = refine_columns(&samples);
        assert_eq!(conversions.len(), 2);
        let clause = replace_clause(&conversions).unwrap();
        assert!(clause.starts_with("* REPLACE ("), "{clause}");
        assert!(clause.contains("AS \"price\""), "{clause}");
        assert!(clause.contains("AS \"when\""), "{clause}");
        assert!(!clause.contains("\"note\" AS"), "{clause}");
        assert_eq!(replace_clause(&[]), None);
    }
}
