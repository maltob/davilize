//! Self-contained HTML report: one page with every visual, rendered by vega-embed,
//! or (when the report has dashboards) one section per dashboard laid out on a CSS grid.
//!
//! Escaping: titles, notes, summaries, table cells and any other text in the markup go
//! through [`escape_html`]. Specs (which hold data) are embedded as JavaScript with
//! [`json_for_script`], which escapes `<`, `>`, `&` and the JS line separators, so
//! no data value can close the `<script>` element or open a comment.
//!
//! Each chart is a `role="img"` element whose `aria-label` is a summary of its data
//! (see [`crate::summary`]), and each chart has a collapsed "Data table" beneath it.
//! The Vega title is left out of the embedded spec, because the card already shows
//! the visual's title. Colours for light and dark mode come from CSS variables on the
//! page, which the script reads and applies to the spec's config.

use serde_json::{Value, json};

use crate::summary::{cell_text, chart_summary};
use crate::table::{Cell, Table};
use crate::{RenderError, Rendered};

pub const CDN_VEGA: &str = "https://cdn.jsdelivr.net/npm/vega@6.4.0/build/vega.min.js";
pub const CDN_VEGA_LITE: &str =
    "https://cdn.jsdelivr.net/npm/vega-lite@6.5.0/build/vega-lite.min.js";
pub const CDN_VEGA_EMBED: &str =
    "https://cdn.jsdelivr.net/npm/vega-embed@7.3.0/build/vega-embed.min.js";

/// Rows shown in the collapsed data table under each chart. The rest are left out.
pub const DATA_TABLE_ROWS: usize = 200;

/// How the page loads vega, vega-lite and vega-embed.
pub enum Scripts<'a> {
    /// Pinned versions from cdn.jsdelivr.net (needs network when the page is opened).
    Cdn,
    /// Library source inlined into the page (works offline). Sources must be the
    /// UMD builds named in the constants above (`build/*.min.js`).
    Inline {
        vega: &'a str,
        vega_lite: &'a str,
        vega_embed: &'a str,
    },
}

/// A dashboard ready for the HTML page: its grid and the charts placed on it.
#[derive(Debug, Clone)]
pub struct DashboardView {
    pub id: String,
    pub title: String,
    pub columns: u32,
    /// Height of one grid row in pixels.
    pub row_height: u32,
    pub charts: Vec<ChartPlacement>,
}

/// A rendered chart at a grid position: column `x` and row `y` (0-based), spanning
/// `w` columns and `h` rows.
#[derive(Debug, Clone)]
pub struct ChartPlacement {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub chart: Rendered,
}

/// Renders the full report page with every visual, in report order.
pub fn html_report(
    report_name: &str,
    visuals: &[Rendered],
    scripts: &Scripts<'_>,
) -> Result<String, RenderError> {
    let specs: Vec<Value> = visuals.iter().map(|v| page_spec(&v.spec)).collect();

    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", escape_html(report_name)));
    if visuals.is_empty() {
        body.push_str("<p class=\"note\">This report has no charts.</p>\n");
    }
    for (index, visual) in visuals.iter().enumerate() {
        body.push_str("<section class=\"visual\">\n");
        body.push_str(&format!("<h2>{}</h2>\n", escape_html(&visual.title)));
        body.push_str(&chart_div(index, visual, "chart"));
        for note in &visual.notes {
            body.push_str(&format!("<p class=\"note\">{}</p>\n", escape_html(note)));
        }
        body.push_str(&data_table(&visual.table));
        body.push_str("</section>\n");
    }

    page(report_name, &body, specs, scripts, "")
}

/// Renders a report page with one section per dashboard. Each dashboard is a CSS
/// grid: items are placed with `grid-column` and `grid-row`, and each chart fills its
/// cell. At 700px wide or less the grid becomes one column, with the cells stacked
/// in reading order (top to bottom, then left to right).
pub fn html_dashboards(
    report_name: &str,
    dashboards: &[DashboardView],
    scripts: &Scripts<'_>,
) -> Result<String, RenderError> {
    let mut body = format!("<h1>{}</h1>\n", escape_html(report_name));
    if dashboards.is_empty() {
        body.push_str("<p class=\"note\">This report has no dashboards.</p>\n");
    }
    // One chart per placement, numbered in page order so each gets its own element id.
    let mut specs = Vec::new();
    for dashboard in dashboards {
        body.push_str(&format!(
            "<section class=\"dashboard\" id=\"dashboard-{}\">\n<h2>{}</h2>\n",
            escape_html(&dashboard.id),
            escape_html(&dashboard.title)
        ));
        body.push_str(&format!(
            "<div class=\"dash-grid\" style=\"grid-template-columns: repeat({columns}, 1fr); grid-auto-rows: {row}px; --row-px: {row}px\">\n",
            columns = dashboard.columns,
            row = dashboard.row_height
        ));

        let mut order: Vec<&ChartPlacement> = dashboard.charts.iter().collect();
        order.sort_by_key(|p| (p.y, p.x));
        for placement in order {
            let index = specs.len();
            let chart = &placement.chart;
            let (spec, fills) = dashboard_spec(&chart.spec);
            specs.push(spec);
            body.push_str(&format!(
                "<div class=\"dash-item\" style=\"grid-column: {x} / span {w}; grid-row: {y} / span {h}; --rows: {h}\">\n",
                x = placement.x.saturating_add(1),
                y = placement.y.saturating_add(1),
                w = placement.w,
                h = placement.h
            ));
            body.push_str(&format!("<h3>{}</h3>\n", escape_html(&chart.title)));
            body.push_str(&chart_div(
                index,
                chart,
                if fills { "chart" } else { "chart natural" },
            ));
            for note in &chart.notes {
                body.push_str(&format!("<p class=\"note\">{}</p>\n", escape_html(note)));
            }
            body.push_str(&data_table(&chart.table));
            body.push_str("</div>\n");
        }
        body.push_str("</div>\n</section>\n");
    }

    page(report_name, &body, specs, scripts, DASHBOARD_CSS)
}

/// The element a chart is drawn into: an image to assistive technology, named by its summary.
fn chart_div(index: usize, chart: &Rendered, class: &str) -> String {
    format!(
        "<div class=\"{class}\" id=\"chart-{index}\" role=\"img\" aria-label=\"{}\"></div>\n",
        escape_html(&chart_summary(chart))
    )
}

/// A collapsed "Data table" with the chart's values, first [`DATA_TABLE_ROWS`] rows.
/// Numbers are right-aligned. Column headers are marked `scope="col"`.
pub fn data_table(table: &Table) -> String {
    let mut out = String::from(
        "<details class=\"data\">\n<summary>Data table</summary>\n<div class=\"data-scroll\">\n<table>\n<thead><tr>",
    );
    for column in &table.columns {
        out.push_str(&format!(
            "<th scope=\"col\">{}</th>",
            escape_html(&column.name)
        ));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in table.rows.iter().take(DATA_TABLE_ROWS) {
        out.push_str("<tr>");
        for cell in row {
            let class = if matches!(cell, Cell::Number(_)) {
                " class=\"num\""
            } else {
                ""
            };
            out.push_str(&format!(
                "<td{class}>{}</td>",
                escape_html(&cell_text(Some(cell)))
            ));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n</div>\n");
    if table.rows.len() > DATA_TABLE_ROWS {
        out.push_str(&format!(
            "<p class=\"note\">Showing first {DATA_TABLE_ROWS} of {} rows.</p>\n",
            table.rows.len()
        ));
    }
    out.push_str("</details>\n");
    out
}

/// Styles added only to pages that have dashboards, so other pages are unchanged.
const DASHBOARD_CSS: &str = "\
.dashboard { margin-top: 1.5em; }
.dash-grid { display: grid; gap: 12px; }
.dash-item { min-width: 0; min-height: 0; display: flex; flex-direction: column; border: 1px solid var(--page-rule); border-radius: 6px; padding: 8px; overflow: hidden; }
.dash-item h3 { font-size: 1rem; margin: 0 0 4px; }
.dash-item .chart { flex: 1 1 auto; min-height: 0; overflow: hidden; }
.dash-item .chart.natural { overflow: auto; }
.dash-item .data-scroll { max-height: 160px; }
@media (max-width: 700px) {
  .dash-grid { grid-template-columns: 1fr !important; grid-auto-rows: auto !important; }
  .dash-item { grid-column: auto !important; grid-row: auto !important; height: calc(var(--rows) * var(--row-px)); }
}
";

/// The spec as the page embeds it: the Vega title is dropped, since the card shows it.
fn page_spec(spec: &Value) -> Value {
    let mut spec = spec.clone();
    if let Value::Object(map) = &mut spec {
        map.remove("title");
    }
    spec
}

/// The spec of a chart inside a dashboard cell: a numeric width or height becomes
/// `"container"`, so the chart fills the cell, and autosize refits it on resize.
/// Also says whether the chart was resized to fill the cell; charts that keep
/// their natural size (tables) are marked so the cell can scroll them.
fn dashboard_spec(spec: &Value) -> (Value, bool) {
    let mut spec = page_spec(spec);
    let mut fills = false;
    if let Value::Object(map) = &mut spec {
        for key in ["width", "height"] {
            if map.get(key).is_some_and(Value::is_number) {
                map.insert(key.to_string(), json!("container"));
                fills = true;
            }
        }
        if fills {
            map.insert(
                "autosize".to_string(),
                json!({"type": "fit", "contains": "padding", "resize": true}),
            );
        }
    }
    (spec, fills)
}

/// Assembles the page around a body. `extra_css` is appended to the style block.
fn page(
    report_name: &str,
    body: &str,
    specs: Vec<Value>,
    scripts: &Scripts<'_>,
    extra_css: &str,
) -> Result<String, RenderError> {
    let specs_js = json_for_script(&Value::Array(specs));
    let libraries = library_tags(scripts)?;
    Ok(format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n\
<style>\n\
{THEME_CSS}\
body {{ font-family: system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif; color: var(--page-text); background: var(--page-bg); margin: 0; }}\n\
main {{ max-width: 980px; margin: 0 auto; padding: 16px; }}\n\
h1 {{ font-size: 1.6rem; margin: 0.5em 0; }}\n\
h2 {{ font-size: 1.15rem; margin: 1.2em 0 0.4em; }}\n\
.visual {{ border-top: 1px solid var(--page-rule); padding-bottom: 8px; }}\n\
.chart {{ overflow-x: auto; max-width: 100%; }}\n\
.note {{ color: var(--page-muted); font-size: 0.875rem; margin: 4px 0; }}\n\
.error {{ color: var(--error); }}\n\
details.data {{ margin: 6px 0; font-size: 0.875rem; }}\n\
details.data summary {{ cursor: pointer; min-height: 28px; display: flex; align-items: center; }}\n\
details.data .data-scroll {{ max-height: 320px; overflow: auto; }}\n\
details.data table {{ border-collapse: collapse; width: 100%; }}\n\
details.data th, details.data td {{ text-align: left; padding: 4px 8px; border-bottom: 1px solid var(--page-rule); vertical-align: top; }}\n\
details.data th {{ position: sticky; top: 0; background: var(--page-bg); }}\n\
details.data td.num {{ text-align: right; font-variant-numeric: tabular-nums; }}\n\
details.data tbody tr:nth-child(even) {{ background: var(--page-zebra); }}\n\
{extra_css}\
</style>\n\
</head>\n\
<body>\n\
<main>\n\
{body}\
</main>\n\
{libraries}\
<script>\n\
var SPECS = {specs_js};\n\
{THEME_JS}\
(function () {{\n\
  if (typeof vegaEmbed !== \"function\") {{\n\
    document.querySelectorAll(\".chart\").forEach(function (el) {{\n\
      el.textContent = \"Charting libraries did not load; check the network or use an offline build.\";\n\
      el.classList.add(\"error\");\n\
    }});\n\
    return;\n\
  }}\n\
  SPECS.forEach(function (spec, i) {{\n\
    var el = document.getElementById(\"chart-\" + i);\n\
    spec.config = mergeConfig(chartTheme(), spec.config || {{}});\n\
    vegaEmbed(el, spec, {{ actions: false }}).catch(function (err) {{\n\
      el.textContent = \"Could not render chart: \" + err;\n\
      el.classList.add(\"error\");\n\
    }});\n\
  }});\n\
}})();\n\
</script>\n\
</body>\n\
</html>\n",
        title = escape_html(report_name),
    ))
}

/// Colour tokens for the page and its charts. Light values first, then dark for users
/// who prefer dark mode, unless the page says `data-theme="light"`.
const THEME_CSS: &str = "\
:root { --page-bg: #ffffff; --page-text: #1f2933; --page-muted: #616e7c; --page-rule: #e4e7eb; --page-zebra: #f5f7fa; --error: #b42318; --chart-text: #1f2933; --chart-muted: #616e7c; --chart-grid: #e4e7eb; --chart-line: #9aa5b1; }
@media (prefers-color-scheme: dark) {
  :root:not([data-theme=\"light\"]) { --page-bg: #0f141b; --page-text: #e6ebf1; --page-muted: #9aa6b4; --page-rule: #2e3a48; --page-zebra: #151c25; --error: #ff8a80; --chart-text: #e6ebf1; --chart-muted: #9aa6b4; --chart-grid: #2e3a48; --chart-line: #5d6b7a; }
}
:root[data-theme=\"dark\"] { --page-bg: #0f141b; --page-text: #e6ebf1; --page-muted: #9aa6b4; --page-rule: #2e3a48; --page-zebra: #151c25; --error: #ff8a80; --chart-text: #e6ebf1; --chart-muted: #9aa6b4; --chart-grid: #2e3a48; --chart-line: #5d6b7a; }
";

/// Reads the chart colour tokens and merges them into a spec's config. The spec's own
/// config wins where both set a key. Matches `chartTheme` and `mergeConfig` in `chartText.ts`.
const THEME_JS: &str = "\
function chartTheme() {
  var style = getComputedStyle(document.documentElement);
  function token(name) { return style.getPropertyValue(name).trim(); }
  var text = token(\"--chart-text\"), line = token(\"--chart-line\"), grid = token(\"--chart-grid\"), muted = token(\"--chart-muted\");
  return {
    axis: { labelColor: text, titleColor: text, domainColor: line, tickColor: line, gridColor: grid },
    legend: { labelColor: text, titleColor: text },
    title: { color: text, subtitleColor: muted }
  };
}
function mergeConfig(target, source) {
  Object.keys(source).forEach(function (key) {
    var value = source[key];
    var plain = value !== null && typeof value === \"object\" && !Array.isArray(value);
    if (plain && target[key] !== null && typeof target[key] === \"object\") {
      mergeConfig(target[key], value);
    } else {
      target[key] = value;
    }
  });
  return target;
}
";

fn library_tags(scripts: &Scripts<'_>) -> Result<String, RenderError> {
    match scripts {
        Scripts::Cdn => Ok(format!(
            "<script src=\"{CDN_VEGA}\"></script>\n\
<script src=\"{CDN_VEGA_LITE}\"></script>\n\
<script src=\"{CDN_VEGA_EMBED}\"></script>\n"
        )),
        Scripts::Inline {
            vega,
            vega_lite,
            vega_embed,
        } => {
            let mut out = String::new();
            for (name, source) in [
                ("vega", vega),
                ("vega-lite", vega_lite),
                ("vega-embed", vega_embed),
            ] {
                check_inline_source(name, source)?;
                out.push_str(&format!("<script>\n{source}\n</script>\n"));
            }
            Ok(out)
        }
    }
}

/// Inlined library code must not be able to end its `<script>` element.
fn check_inline_source(name: &str, source: &str) -> Result<(), RenderError> {
    let lower = source.to_ascii_lowercase();
    if lower.contains("</script") || source.contains("<!--") {
        return Err(RenderError::Output(format!(
            "vendored {name} contains markup that cannot be inlined safely"
        )));
    }
    Ok(())
}

/// Serializes a JSON value so it can sit inside a `<script>` element.
pub fn json_for_script(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "null".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Escapes text for HTML element content and double-quoted attributes.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(c),
        }
    }
    out
}
