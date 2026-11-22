//! Self-contained HTML report: one page with every visual, rendered by vega-embed.
//!
//! Escaping: titles, notes and any other text in the markup go through
//! [`escape_html`]. Specs (which hold data) are embedded as JavaScript with
//! [`json_for_script`], which escapes `<`, `>`, `&` and the JS line separators, so
//! no data value can close the `<script>` element or open a comment.

use serde_json::Value;

use crate::{RenderError, Rendered};

pub const CDN_VEGA: &str = "https://cdn.jsdelivr.net/npm/vega@6.4.0/build/vega.min.js";
pub const CDN_VEGA_LITE: &str =
    "https://cdn.jsdelivr.net/npm/vega-lite@6.5.0/build/vega-lite.min.js";
pub const CDN_VEGA_EMBED: &str =
    "https://cdn.jsdelivr.net/npm/vega-embed@7.3.0/build/vega-embed.min.js";

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

/// Renders the full report page.
pub fn html_report(
    report_name: &str,
    visuals: &[Rendered],
    scripts: &Scripts<'_>,
) -> Result<String, RenderError> {
    let specs: Vec<Value> = visuals.iter().map(|v| v.spec.clone()).collect();
    let specs_js = json_for_script(&Value::Array(specs));

    let mut body = String::new();
    body.push_str(&format!("<h1>{}</h1>\n", escape_html(report_name)));
    if visuals.is_empty() {
        body.push_str("<p class=\"note\">This report has no visuals.</p>\n");
    }
    for (index, visual) in visuals.iter().enumerate() {
        body.push_str("<section class=\"visual\">\n");
        body.push_str(&format!("<h2>{}</h2>\n", escape_html(&visual.title)));
        body.push_str(&format!(
            "<div class=\"chart\" id=\"chart-{index}\"></div>\n"
        ));
        for note in &visual.notes {
            body.push_str(&format!("<p class=\"note\">{}</p>\n", escape_html(note)));
        }
        body.push_str("</section>\n");
    }

    let libraries = library_tags(scripts)?;
    Ok(format!(
        "<!doctype html>\n\
<html lang=\"en\">\n\
<head>\n\
<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
<title>{title}</title>\n\
<style>\n\
body {{ font-family: system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif; color: #1f2933; background: #ffffff; margin: 0; }}\n\
main {{ max-width: 980px; margin: 0 auto; padding: 16px; }}\n\
h1 {{ font-size: 1.6rem; margin: 0.5em 0; }}\n\
h2 {{ font-size: 1.15rem; margin: 1.2em 0 0.4em; }}\n\
.visual {{ border-top: 1px solid #e4e7eb; padding-bottom: 8px; }}\n\
.chart {{ overflow-x: auto; max-width: 100%; }}\n\
.note {{ color: #616e7c; font-size: 0.875rem; margin: 4px 0; }}\n\
.error {{ color: #b42318; }}\n\
</style>\n\
</head>\n\
<body>\n\
<main>\n\
{body}\
</main>\n\
{libraries}\
<script>\n\
var SPECS = {specs_js};\n\
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
