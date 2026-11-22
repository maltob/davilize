//! Chart style shared by the Vega-Lite specs, the HTML report and the native SVG
//! charts, so every output agrees: the categorical palette, the marker shapes, and
//! the theme-neutral Vega-Lite config. Colours that depend on light or dark mode
//! are not set here; the page or the app layers them on from its CSS variables.

use serde_json::{Value, json};

/// Adjusted Okabe-Ito palette, in the order series take colours. The canonical
/// colours are darkened where needed so every one reaches 3:1 contrast against
/// white and against the app's dark surface (#171e27). Black is replaced by grey,
/// which would disappear on a dark background.
pub const PALETTE: [&str; 8] = [
    "#b87400", // orange
    "#2a7db0", // sky blue
    "#009e73", // bluish green
    "#a88f00", // yellow
    "#0072b2", // blue
    "#d55e00", // vermillion
    "#bc6c9e", // reddish purple
    "#6e7782", // grey (for black)
];

/// Marker shapes for line points and scatter points, one per palette colour. A series
/// is told apart by shape as well as colour, so colour is never the only cue.
pub const SHAPES: [&str; 8] = [
    "circle",
    "square",
    "triangle-up",
    "diamond",
    "triangle-down",
    "cross",
    "triangle-right",
    "triangle-left",
];

/// Font stack shared with the app's CSS.
pub const FONT: &str = "system-ui, -apple-system, \"Segoe UI\", Roboto, sans-serif";

/// The Vega-Lite config every spec carries. It has no background colour, so a chart
/// takes the colour of the card or page around it, in light and dark mode alike.
pub fn chart_config() -> Value {
    json!({
        "background": "transparent",
        "font": FONT,
        "view": {"stroke": null},
        "range": {"category": PALETTE},
        "axis": {"labelFontSize": 12, "titleFontSize": 13, "titlePadding": 10},
        "axisX": {"labelAngle": 0, "labelOverlap": "parity", "labelLimit": 140},
        "legend": {"labelFontSize": 12, "titleFontSize": 13},
        "title": {"fontSize": 15, "anchor": "start", "subtitleFontSize": 12}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG relative luminance of a #rrggbb colour.
    fn luminance(hex: &str) -> f64 {
        let channel = |i: usize| {
            let v = f64::from(u8::from_str_radix(&hex[i..i + 2], 16).unwrap()) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(1) + 0.7152 * channel(3) + 0.0722 * channel(5)
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn every_palette_colour_has_3_to_1_contrast_on_light_and_dark() {
        for colour in PALETTE {
            assert!(contrast(colour, "#ffffff") >= 3.0, "{colour} on white");
            assert!(contrast(colour, "#171e27") >= 3.0, "{colour} on dark");
        }
    }

    #[test]
    fn palette_has_one_shape_per_colour() {
        assert_eq!(PALETTE.len(), SHAPES.len());
        let mut shapes = SHAPES.to_vec();
        shapes.sort_unstable();
        shapes.dedup();
        assert_eq!(shapes.len(), SHAPES.len());
    }

    #[test]
    fn config_has_no_dark_theme_or_fixed_text_colour() {
        let config = chart_config();
        assert_eq!(config["background"], json!("transparent"));
        assert_eq!(config["range"]["category"][0], json!(PALETTE[0]));
        // Text and axis colours come from the page, so they must not be set here.
        assert!(config["axis"].get("labelColor").is_none());
        assert!(config.get("title").and_then(|t| t.get("color")).is_none());
    }
}
