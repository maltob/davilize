//! Plain-text summaries of a chart, used as its accessible name (`aria-label`) in the
//! HTML report. The app builds the same kind of text from its spec in `chartText.ts`.
//! The caller HTML-escapes the result; this module only builds the words.

use dv_core::ChartType;

use crate::Rendered;
use crate::table::{Cell, Table};

/// Longest summary kept. Longer text is cut and ends with "...".
pub const MAX_SUMMARY_CHARS: usize = 250;

/// A one-line description of a chart: its type, what it plots, its size and its
/// highest and lowest values. For example "Bar chart: total_revenue by region. 4 bars.
/// Highest: North 735, lowest: West 120."
pub fn chart_summary(rendered: &Rendered) -> String {
    let text = match rendered.chart {
        ChartType::Bar => categorical(rendered, "Bar chart", "bar", "bars"),
        ChartType::Line => categorical(rendered, "Line chart", "point", "points"),
        ChartType::Scatter => scatter(rendered),
        ChartType::Heatmap => heatmap(rendered),
        ChartType::Table => table(rendered),
    };
    cap(&text)
}

/// Bar and line charts: the y measure by the x category, with the highest and lowest values.
fn categorical(rendered: &Rendered, kind: &str, one: &str, many: &str) -> String {
    let table = &rendered.table;
    let enc = &rendered.encodings;
    let x = column(table, enc.x.as_deref());
    let colour = column(table, enc.color.as_deref());
    let y = column(table, enc.y.as_deref());
    let mut out = format!(
        "{kind}: {} by {}. {}.",
        name_or(&enc.y, "value"),
        name_or(&enc.x, "category"),
        count(table.rows.len(), one, many)
    );
    let found = extremes(table, y, |row| {
        let label = cell_text(row.get(x.unwrap_or(usize::MAX)));
        match colour {
            Some(c) => format!("{label} ({})", cell_text(row.get(c))),
            None => label,
        }
    });
    push_extremes(&mut out, found);
    out
}

fn scatter(rendered: &Rendered) -> String {
    let table = &rendered.table;
    let enc = &rendered.encodings;
    let x_name = name_or(&enc.x, "x");
    let y_name = name_or(&enc.y, "y");
    let mut out = format!(
        "Scatter plot: {y_name} against {x_name}. {}.",
        count(table.rows.len(), "point", "points")
    );
    if let (Some((x_lo, x_hi)), Some((y_lo, y_hi))) = (
        range(table, column(table, enc.x.as_deref())),
        range(table, column(table, enc.y.as_deref())),
    ) {
        out.push_str(&format!(
            " {x_name} from {} to {}, {y_name} from {} to {}.",
            fmt_number(x_lo),
            fmt_number(x_hi),
            fmt_number(y_lo),
            fmt_number(y_hi)
        ));
    }
    out
}

fn heatmap(rendered: &Rendered) -> String {
    let table = &rendered.table;
    let enc = &rendered.encodings;
    let x = column(table, enc.x.as_deref());
    let y = column(table, enc.y.as_deref());
    let colour = column(table, enc.color.as_deref());
    let mut out = format!(
        "Heatmap: {} by {} and {}. {}.",
        name_or(&enc.color, "value"),
        name_or(&enc.x, "x"),
        name_or(&enc.y, "y"),
        count(table.rows.len(), "cell", "cells")
    );
    let found = extremes(table, colour, |row| {
        format!(
            "{}, {}",
            cell_text(row.get(x.unwrap_or(usize::MAX))),
            cell_text(row.get(y.unwrap_or(usize::MAX)))
        )
    });
    push_extremes(&mut out, found);
    out
}

fn table(rendered: &Rendered) -> String {
    let table = &rendered.table;
    let names: Vec<&str> = table.columns.iter().map(|c| c.name.as_str()).collect();
    let mut out = format!("Table: {}.", count(table.rows.len(), "row", "rows"));
    if !names.is_empty() {
        out.push_str(&format!(" Columns: {}.", names.join(", ")));
    }
    out
}

/// Text of one cell as shown in the data table and in summaries.
pub(crate) fn cell_text(cell: Option<&Cell>) -> String {
    match cell {
        None | Some(Cell::Null) => String::new(),
        Some(Cell::Number(n)) => fmt_number(*n),
        Some(Cell::Bool(b)) => b.to_string(),
        Some(Cell::Text(s)) => s.clone(),
        Some(Cell::Time { text, .. }) => text.clone(),
    }
}

/// Formats a number with at most two decimals, dropping trailing zeros.
pub(crate) fn fmt_number(v: f64) -> String {
    if !v.is_finite() {
        return "n/a".to_string();
    }
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn column(table: &Table, name: Option<&str>) -> Option<usize> {
    name.and_then(|n| table.column_index(n))
}

fn name_or(name: &Option<String>, fallback: &str) -> String {
    name.clone().unwrap_or_else(|| fallback.to_string())
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The highest and lowest finite values of `value`, each with its label.
type Extremes = ((String, f64), (String, f64));

fn extremes(
    table: &Table,
    value: Option<usize>,
    label: impl Fn(&[Cell]) -> String,
) -> Option<Extremes> {
    let index = value?;
    let mut high: Option<(String, f64)> = None;
    let mut low: Option<(String, f64)> = None;
    for row in &table.rows {
        let Some(Cell::Number(v)) = row.get(index) else {
            continue;
        };
        if !v.is_finite() {
            continue;
        }
        let text = label(row.as_slice());
        if high.as_ref().is_none_or(|(_, h)| v > h) {
            high = Some((text.clone(), *v));
        }
        if low.as_ref().is_none_or(|(_, l)| v < l) {
            low = Some((text, *v));
        }
    }
    Some((high?, low?))
}

fn push_extremes(out: &mut String, found: Option<Extremes>) {
    if let Some(((high_label, high), (low_label, low))) = found {
        out.push_str(&format!(
            " Highest: {high_label} {}, lowest: {low_label} {}.",
            fmt_number(high),
            fmt_number(low)
        ));
    }
}

/// Smallest and largest finite value in a column, if there is one.
fn range(table: &Table, index: Option<usize>) -> Option<(f64, f64)> {
    let index = index?;
    let mut values = table.rows.iter().filter_map(|row| match row.get(index) {
        Some(Cell::Number(v)) if v.is_finite() => Some(*v),
        _ => None,
    });
    let first = values.next()?;
    Some(values.fold((first, first), |(lo, hi), v| (lo.min(v), hi.max(v))))
}

/// Cuts text to [`MAX_SUMMARY_CHARS`] characters, ending with "..." when cut.
fn cap(text: &str) -> String {
    if text.chars().count() <= MAX_SUMMARY_CHARS {
        return text.to_string();
    }
    let kept: String = text.chars().take(MAX_SUMMARY_CHARS - 3).collect();
    format!("{}...", kept.trim_end())
}
