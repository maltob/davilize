//! Small helpers shared by the Tableau and Power BI converters.

use std::collections::HashSet;

use dv_core::{Dashboard, DashboardItem, Dataset};

/// True for a path that is absolute on Windows (`C:/data`, `C:\data`, `//server/share`)
/// or on Unix (`/data`). A workbook saved on Windows holds Windows paths, and they must
/// not be taken as relative just because this machine is not Windows.
pub fn is_absolute_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\');
    drive
        || path.starts_with('/')
        || path.starts_with("\\\\")
        || std::path::Path::new(path).is_absolute()
}

/// Turns any text into a valid Davilize name (letters, digits, `_` and `-`).
pub fn sanitize(text: &str) -> String {
    let mut out: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

/// Hands out unique names: the second `sales` becomes `sales_2`.
#[derive(Default)]
pub struct UniqueNames {
    used: HashSet<String>,
}

impl UniqueNames {
    pub fn claim(&mut self, wanted: &str) -> String {
        let base = sanitize(wanted);
        let mut name = base.clone();
        let mut n = 2;
        while !self.used.insert(name.to_ascii_lowercase()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        name
    }
}

/// Escapes text for XML attributes and element content.
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Clamps items into the grid and moves overlapping ones down, so the dashboard
/// passes validation whatever the source layout looked like.
pub fn fit_items(dashboard: &mut Dashboard) {
    let columns = dashboard.columns.max(1);
    let mut placed: Vec<DashboardItem> = Vec::new();
    let mut items = std::mem::take(&mut dashboard.items);
    items.sort_by_key(|i| (i.y, i.x));
    for mut item in items {
        item.w = item.w.clamp(1, columns);
        item.h = item.h.max(1);
        if item.x + item.w > columns {
            item.x = columns - item.w;
        }
        while let Some(hit) = placed.iter().find(|p| overlap(p, &item)) {
            item.y = hit.y + hit.h;
        }
        placed.push(item);
    }
    dashboard.items = placed;
}

fn overlap(a: &DashboardItem, b: &DashboardItem) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// Converts a position or size in `0..total` units to grid units.
pub fn to_grid(value: f64, total: f64, cells: u32) -> u32 {
    if total <= 0.0 {
        return 0;
    }
    ((value / total) * f64::from(cells)).round().max(0.0) as u32
}

/// Follows a dataset down through datasets built on datasets. Returns the chain
/// base-first and the name of the base source, or why it cannot be flattened.
pub fn dataset_chain<'a>(
    report: &'a dv_core::Report,
    dataset: &'a Dataset,
) -> Result<(Vec<&'a Dataset>, &'a str), String> {
    let mut chain = vec![dataset];
    let mut current = dataset;
    loop {
        if current.sql.is_some() {
            return Err(format!("dataset '{}' uses custom SQL", current.name));
        }
        match report.datasets.iter().find(|d| d.name == current.source) {
            Some(parent) => {
                chain.push(parent);
                current = parent;
            }
            None => break,
        }
    }
    chain.reverse();
    Ok((chain, current.source.as_str()))
}

#[cfg(test)]
mod path_tests {
    use super::is_absolute_path;

    #[test]
    fn windows_and_unix_absolute_paths_are_recognised_on_every_os() {
        for absolute in [
            "C:/data/a.csv",
            r"c:\data\a.csv",
            "//server/share/a.csv",
            r"\\server\share\a.csv",
            "/data/a.csv",
        ] {
            assert!(is_absolute_path(absolute), "{absolute}");
        }
        for relative in ["a.csv", "data/a.csv", r"..\a.csv", "C:a.csv", "./a.csv"] {
            assert!(!is_absolute_path(relative), "{relative}");
        }
    }
}
