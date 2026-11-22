//! Native SVG renderer for bar, line, scatter, table and heatmap charts.
//!
//! Every chart type has a renderer here (they return
//! [`RenderError::Unsupported`]); use the HTML report for them. The renderer
//! draws axes with "nice" linear ticks, categorical positions for non-numeric
//! x values (in first-seen order, so the query's sort order is kept), and a
//! colour legend for the colour encoding. Size maps to radius for scatter points.

use dv_core::ChartType;

use crate::html::escape_html;
use crate::table::Cell;
use crate::{Channels, RenderError, Rendered, required};

const WIDTH: f64 = 800.0;
const HEIGHT: f64 = 500.0;
const LEFT: f64 = 84.0;
const RIGHT: f64 = 180.0;
const TOP: f64 = 56.0;
const BOTTOM: f64 = 96.0;
const PALETTE: [&str; 10] = [
    "#4e79a7", "#f28e2b", "#e15759", "#76b7b2", "#59a14f", "#edc948", "#b07aa1", "#ff9da7",
    "#9c755f", "#bab0ac",
];
const MAX_LEGEND: usize = 20;

/// Renders a bar, line or scatter chart as a standalone SVG document.
pub fn render(rendered: &Rendered) -> Result<String, RenderError> {
    let (chart, kind) = match rendered.chart {
        ChartType::Bar => ("bar", ChartType::Bar),
        ChartType::Line => ("line", ChartType::Line),
        ChartType::Scatter => ("scatter", ChartType::Scatter),
        ChartType::Table => return render_table(rendered),
        ChartType::Heatmap => return render_heatmap(rendered),
    };
    let channels = Channels::resolve(&rendered.id, &rendered.encodings, &rendered.table)?;
    let x = required(&rendered.id, chart, "x", &channels.x)?;
    let y = required(&rendered.id, chart, "y", &channels.y)?;
    let table = &rendered.table;
    let xi = table.column_index(&x.name);
    let yi = table.column_index(&y.name);
    let ci = channels
        .color
        .as_ref()
        .and_then(|f| table.column_index(&f.name));
    let si = channels
        .size
        .as_ref()
        .and_then(|f| table.column_index(&f.name));

    // Collect plottable points: y must be a number; x is a number for linear
    // axes or a label for categorical ones.
    let linear_x = kind != ChartType::Bar && x.kind == crate::Kind::Quantitative;
    let mut categories: Vec<String> = Vec::new();
    let mut series: Vec<String> = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    for row in &table.rows {
        let Some(value_y) = number(get(row, yi)) else {
            continue;
        };
        let x_pos = if linear_x {
            match number(get(row, xi)) {
                Some(v) => v,
                None => continue,
            }
        } else {
            let name = label(get(row, xi));
            match categories.iter().position(|c| *c == name) {
                Some(i) => i as f64,
                None => {
                    categories.push(name);
                    (categories.len() - 1) as f64
                }
            }
        };
        let series_index = if channels.color.is_some() {
            let name = label(get(row, ci));
            Some(match series.iter().position(|s| *s == name) {
                Some(i) => i,
                None => {
                    series.push(name);
                    series.len() - 1
                }
            })
        } else {
            None
        };
        let size = number(get(row, si));
        points.push(Point {
            x: x_pos,
            y: value_y,
            series: series_index,
            size,
        });
    }

    // Vega-Lite orders a nominal colour domain alphabetically, not by first
    // appearance. Match it so a series gets the same colour in the app preview
    // and in the exported SVG/PNG/PDF.
    let mut sorted = series.clone();
    sorted.sort();
    for point in &mut points {
        if let Some(i) = point.series {
            point.series = sorted.iter().position(|s| *s == series[i]);
        }
    }
    let series = sorted;

    let (y_lo, y_hi) = domain(points.iter().map(|p| p.y), kind == ChartType::Bar);
    let (y_start, y_end, y_ticks) = nice_ticks(y_lo, y_hi, 5);
    let (x_start, x_end, x_ticks) = if linear_x {
        let (lo, hi) = domain(points.iter().map(|p| p.x), false);
        nice_ticks(lo, hi, 6)
    } else {
        (0.0, categories.len() as f64, Vec::new())
    };

    let plot = Plot {
        left: LEFT,
        right: WIDTH - RIGHT,
        top: TOP,
        bottom: HEIGHT - BOTTOM,
    };
    let y_px = |v: f64| plot.bottom - (v - y_start) / (y_end - y_start) * (plot.bottom - plot.top);
    let x_px = |v: f64| {
        if linear_x {
            plot.left + (v - x_start) / (x_end - x_start) * (plot.right - plot.left)
        } else {
            let band = (plot.right - plot.left) / categories.len().max(1) as f64;
            plot.left + (v + 0.5) * band
        }
    };
    let band = (plot.right - plot.left) / categories.len().max(1) as f64;

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" font-family=\"sans-serif\">\n"
    ));
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"#ffffff\"/>\n"
    ));
    out.push_str(&format!(
        "<text x=\"{LEFT}\" y=\"32\" font-size=\"18\" font-weight=\"bold\" fill=\"#1f2933\">{}</text>\n",
        escape_html(&rendered.title)
    ));

    // Gridlines and y ticks.
    for tick in &y_ticks {
        let py = y_px(*tick);
        out.push_str(&format!(
            "<line x1=\"{:.2}\" y1=\"{py:.2}\" x2=\"{:.2}\" y2=\"{py:.2}\" stroke=\"#e4e7eb\"/>\n",
            plot.left, plot.right
        ));
        out.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12\" text-anchor=\"end\" fill=\"#616e7c\">{}</text>\n",
            plot.left - 8.0,
            py + 4.0,
            fmt_num(*tick)
        ));
    }

    // Data.
    let series_colour = |i: usize| PALETTE[i % PALETTE.len()];
    match kind {
        ChartType::Bar => {
            let n_series = series.len().max(1) as f64;
            let bar_width = band * 0.8 / n_series;
            let baseline = y_px(0.0);
            for p in &points {
                let cat_center = x_px(p.x);
                let slot = p.series.unwrap_or(0) as f64;
                let left = cat_center - band * 0.4 + slot * bar_width;
                let top = y_px(p.y).min(baseline);
                let height = (y_px(p.y) - baseline).abs();
                out.push_str(&format!(
                    "<rect x=\"{left:.2}\" y=\"{top:.2}\" width=\"{:.2}\" height=\"{height:.2}\" fill=\"{}\"/>\n",
                    bar_width.max(1.0),
                    series_colour(p.series.unwrap_or(0))
                ));
            }
        }
        ChartType::Line => {
            let series_count = series.len().max(1);
            for s in 0..series_count {
                let mut line: Vec<&Point> = points
                    .iter()
                    .filter(|p| p.series.unwrap_or(0) == s)
                    .collect();
                if linear_x {
                    line.sort_by(|a, b| a.x.total_cmp(&b.x));
                }
                if line.is_empty() {
                    continue;
                }
                let coords: Vec<String> = line
                    .iter()
                    .map(|p| format!("{:.2},{:.2}", x_px(p.x), y_px(p.y)))
                    .collect();
                out.push_str(&format!(
                    "<polyline points=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\"/>\n",
                    coords.join(" "),
                    series_colour(s)
                ));
                for p in &line {
                    out.push_str(&format!(
                        "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"3\" fill=\"{}\"/>\n",
                        x_px(p.x),
                        y_px(p.y),
                        series_colour(s)
                    ));
                }
            }
        }
        _ => {
            let (size_lo, size_hi) = domain(points.iter().filter_map(|p| p.size), false);
            for p in &points {
                let radius = match p.size {
                    Some(v) if size_hi > size_lo => {
                        3.0 + 11.0 * ((v - size_lo) / (size_hi - size_lo)).sqrt()
                    }
                    Some(_) => 6.0,
                    None => 4.5,
                };
                out.push_str(&format!(
                    "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{radius:.2}\" fill=\"{}\" fill-opacity=\"0.75\"/>\n",
                    x_px(p.x),
                    y_px(p.y),
                    series_colour(p.series.unwrap_or(0))
                ));
            }
        }
    }

    // Axes.
    out.push_str(&format!(
        "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#9aa5b1\"/>\n",
        plot.left, plot.bottom, plot.right, plot.bottom
    ));
    out.push_str(&format!(
        "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" stroke=\"#9aa5b1\"/>\n",
        plot.left, plot.top, plot.left, plot.bottom
    ));
    if linear_x {
        for tick in x_ticks {
            let px = x_px(tick);
            out.push_str(&format!(
                "<text x=\"{px:.2}\" y=\"{:.2}\" font-size=\"12\" text-anchor=\"middle\" fill=\"#616e7c\">{}</text>\n",
                plot.bottom + 18.0,
                fmt_num(tick)
            ));
        }
    } else {
        let rotate = categories.len() > 12;
        for (i, name) in categories.iter().enumerate() {
            let px = x_px(i as f64);
            let text = escape_html(name);
            if rotate {
                out.push_str(&format!(
                    "<text transform=\"translate({px:.2},{:.2}) rotate(-40)\" font-size=\"11\" text-anchor=\"end\" fill=\"#616e7c\">{text}</text>\n",
                    plot.bottom + 14.0
                ));
            } else {
                out.push_str(&format!(
                    "<text x=\"{px:.2}\" y=\"{:.2}\" font-size=\"12\" text-anchor=\"middle\" fill=\"#616e7c\">{text}</text>\n",
                    plot.bottom + 18.0
                ));
            }
        }
    }
    out.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"13\" text-anchor=\"middle\" fill=\"#1f2933\">{}</text>\n",
        (plot.left + plot.right) / 2.0,
        HEIGHT - 30.0,
        escape_html(&x.name)
    ));
    let mid_y = (plot.top + plot.bottom) / 2.0;
    out.push_str(&format!(
        "<text transform=\"translate(18,{mid_y:.2}) rotate(-90)\" font-size=\"13\" text-anchor=\"middle\" fill=\"#1f2933\">{}</text>\n",
        escape_html(&y.name)
    ));

    // Legend for the colour encoding.
    if channels.color.is_some() && !series.is_empty() {
        let lx = plot.right + 20.0;
        for (i, name) in series.iter().take(MAX_LEGEND).enumerate() {
            let ly = plot.top + 4.0 + i as f64 * 20.0;
            out.push_str(&format!(
                "<rect x=\"{lx:.2}\" y=\"{ly:.2}\" width=\"12\" height=\"12\" fill=\"{}\"/>\n",
                series_colour(i)
            ));
            out.push_str(&format!(
                "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12\" fill=\"#1f2933\">{}</text>\n",
                lx + 18.0,
                ly + 10.0,
                escape_html(name)
            ));
        }
        if series.len() > MAX_LEGEND {
            let ly = plot.top + 4.0 + MAX_LEGEND as f64 * 20.0;
            out.push_str(&format!(
                "<text x=\"{lx:.2}\" y=\"{:.2}\" font-size=\"12\" fill=\"#616e7c\">+{} more</text>\n",
                ly + 10.0,
                series.len() - MAX_LEGEND
            ));
        }
    }

    // Notes (row cap, downsampling).
    for (i, note) in rendered.notes.iter().enumerate() {
        out.push_str(&format!(
            "<text x=\"{LEFT}\" y=\"{:.2}\" font-size=\"11\" fill=\"#616e7c\">{}</text>\n",
            HEIGHT - 10.0 - i as f64 * 14.0,
            escape_html(note)
        ));
    }
    out.push_str("</svg>\n");
    Ok(out)
}

/// Viridis colour stops, matching the `viridis` scheme the Vega-Lite heatmap uses.
const VIRIDIS: [(u8, u8, u8); 9] = [
    (0x44, 0x01, 0x54),
    (0x47, 0x2d, 0x7b),
    (0x3b, 0x52, 0x8b),
    (0x2c, 0x72, 0x8e),
    (0x21, 0x91, 0x8c),
    (0x28, 0xae, 0x80),
    (0x5e, 0xc9, 0x62),
    (0xad, 0xdc, 0x30),
    (0xfd, 0xe7, 0x25),
];
const HEAT_MAX_AXIS: usize = 50;

/// Colour for `t` in `[0, 1]`, linearly interpolated between viridis stops.
fn viridis(t: f64) -> String {
    let t = if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let scaled = t * (VIRIDIS.len() - 1) as f64;
    let i = (scaled.floor() as usize).min(VIRIDIS.len() - 2);
    let f = scaled - i as f64;
    let (a, b) = (VIRIDIS[i], VIRIDIS[i + 1]);
    let mix = |p: u8, q: u8| (p as f64 + (q as f64 - p as f64) * f).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        mix(a.0, b.0),
        mix(a.1, b.1),
        mix(a.2, b.2)
    )
}

/// Index of `name` in `list`, appending it while there is room. `None` when the
/// axis is full and the value is new.
fn axis_index(list: &mut Vec<String>, name: String) -> Option<usize> {
    match list.iter().position(|s| *s == name) {
        Some(i) => Some(i),
        None if list.len() < HEAT_MAX_AXIS => {
            list.push(name);
            Some(list.len() - 1)
        }
        None => None,
    }
}

/// Renders a heatmap: one rect per (x, y) pair coloured on a viridis scale, with a
/// gradient legend. Axis values keep first-seen order, so the query's sort order is
/// kept; each axis shows at most [`HEAT_MAX_AXIS`] values.
fn render_heatmap(rendered: &Rendered) -> Result<String, RenderError> {
    let channels = Channels::resolve(&rendered.id, &rendered.encodings, &rendered.table)?;
    let x = required(&rendered.id, "heatmap", "x", &channels.x)?;
    let y = required(&rendered.id, "heatmap", "y", &channels.y)?;
    let color = required(&rendered.id, "heatmap", "color", &channels.color)?;
    let table = &rendered.table;
    let (xi, yi, ci) = (
        table.column_index(&x.name),
        table.column_index(&y.name),
        table.column_index(&color.name),
    );

    let mut xs: Vec<String> = Vec::new();
    let mut ys: Vec<String> = Vec::new();
    let mut cells: Vec<(usize, usize, f64)> = Vec::new();
    let mut hidden = false;
    for row in &table.rows {
        let Some(value) = number(get(row, ci)) else {
            continue;
        };
        let cx = axis_index(&mut xs, label(get(row, xi)));
        let cy = axis_index(&mut ys, label(get(row, yi)));
        match (cx, cy) {
            (Some(cx), Some(cy)) => cells.push((cx, cy, value)),
            _ => hidden = true,
        }
    }

    let (lo, hi) = domain(cells.iter().map(|c| c.2), false);
    let span = if (hi - lo).abs() < 1e-12 {
        1.0
    } else {
        hi - lo
    };

    let plot = Plot {
        left: LEFT + 20.0,
        right: WIDTH - RIGHT + 40.0,
        top: TOP,
        bottom: HEIGHT - BOTTOM,
    };
    let cw = (plot.right - plot.left) / xs.len().max(1) as f64;
    let ch = (plot.bottom - plot.top) / ys.len().max(1) as f64;

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" font-family=\"Helvetica, Arial, sans-serif\">\n"
    ));
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"#ffffff\"/>\n"
    ));
    out.push_str(&format!(
        "<text x=\"{LEFT}\" y=\"32\" font-size=\"18\" font-weight=\"bold\" fill=\"#1f2933\">{}</text>\n",
        escape_html(&rendered.title)
    ));
    for (cx, cy, value) in &cells {
        out.push_str(&format!(
            "<rect x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\" fill=\"{}\"/>\n",
            plot.left + *cx as f64 * cw,
            plot.top + *cy as f64 * ch,
            cw,
            ch,
            viridis((value - lo) / span)
        ));
    }
    // Axis labels: thin the x labels when they would overlap.
    let x_step = ((xs.len() as f64 * 56.0) / (plot.right - plot.left))
        .ceil()
        .max(1.0) as usize;
    for (i, name) in xs.iter().enumerate().step_by(x_step) {
        out.push_str(&format!(
            "<text transform=\"translate({:.2},{:.2}) rotate(-40)\" font-size=\"11\" text-anchor=\"end\" fill=\"#616e7c\">{}</text>\n",
            plot.left + (i as f64 + 0.5) * cw,
            plot.bottom + 14.0,
            escape_html(&truncate(name, 14))
        ));
    }
    let y_step = ((ys.len() as f64 * 16.0) / (plot.bottom - plot.top))
        .ceil()
        .max(1.0) as usize;
    for (i, name) in ys.iter().enumerate().step_by(y_step) {
        out.push_str(&format!(
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12\" text-anchor=\"end\" fill=\"#616e7c\">{}</text>\n",
            plot.left - 8.0,
            plot.top + (i as f64 + 0.5) * ch + 4.0,
            escape_html(&truncate(name, 14))
        ));
    }
    out.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"13\" text-anchor=\"middle\" fill=\"#1f2933\">{}</text>\n",
        (plot.left + plot.right) / 2.0,
        HEIGHT - 28.0,
        escape_html(&x.name)
    ));
    out.push_str(&format!(
        "<text transform=\"translate(18,{:.2}) rotate(-90)\" font-size=\"13\" text-anchor=\"middle\" fill=\"#1f2933\">{}</text>\n",
        (plot.top + plot.bottom) / 2.0,
        escape_html(&y.name)
    ));

    // Gradient legend with min/max labels.
    let lx = plot.right + 24.0;
    let (ly0, ly1) = (plot.top, plot.top + 160.0);
    out.push_str("<defs><linearGradient id=\"heat\" x1=\"0\" y1=\"1\" x2=\"0\" y2=\"0\">\n");
    for k in 0..=8 {
        let t = k as f64 / 8.0;
        out.push_str(&format!(
            "<stop offset=\"{:.3}\" stop-color=\"{}\"/>\n",
            t,
            viridis(t)
        ));
    }
    out.push_str("</linearGradient></defs>\n");
    out.push_str(&format!(
        "<rect x=\"{lx:.2}\" y=\"{ly0:.2}\" width=\"14\" height=\"{:.2}\" fill=\"url(#heat)\"/>\n",
        ly1 - ly0
    ));
    out.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12\" fill=\"#1f2933\">{}</text>\n",
        lx + 20.0,
        ly0 + 10.0,
        escape_html(&fmt_num(hi))
    ));
    out.push_str(&format!(
        "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"12\" fill=\"#1f2933\">{}</text>\n",
        lx + 20.0,
        ly1,
        escape_html(&fmt_num(lo))
    ));
    out.push_str(&format!(
        "<text x=\"{lx:.2}\" y=\"{:.2}\" font-size=\"12\" fill=\"#1f2933\">{}</text>\n",
        ly0 - 8.0,
        escape_html(&truncate(&color.name, 18))
    ));

    let mut notes = rendered.notes.clone();
    if hidden {
        notes.push(format!("Showing at most {HEAT_MAX_AXIS} values per axis."));
    }
    for (i, note) in notes.iter().enumerate() {
        out.push_str(&format!(
            "<text x=\"{LEFT}\" y=\"{:.2}\" font-size=\"11\" fill=\"#616e7c\">{}</text>\n",
            HEIGHT - 10.0 - i as f64 * 14.0,
            escape_html(note)
        ));
    }
    out.push_str("</svg>\n");
    Ok(out)
}

const TABLE_MAX_ROWS: usize = 40;
const TABLE_ROW_H: f64 = 24.0;
const TABLE_PAD: f64 = 10.0;
const TABLE_CHAR_W: f64 = 7.2;
const TABLE_MIN_COL: f64 = 72.0;
const TABLE_MAX_COL: f64 = 280.0;

/// How many characters fit in a column of `width` (tolerant of float rounding).
fn fit_chars(width: f64) -> usize {
    (((width - 2.0 * TABLE_PAD) / TABLE_CHAR_W) + 1e-6)
        .floor()
        .max(1.0) as usize
}

/// Shortens `text` to `max` characters, ending in an ellipsis when cut.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
        cut.push('\u{2026}');
        cut
    }
}

/// Renders a table visual as a grid: header row, zebra rows, numbers right-aligned.
/// At most [`TABLE_MAX_ROWS`] rows are drawn; the rest are summarised in a note.
fn render_table(rendered: &Rendered) -> Result<String, RenderError> {
    let table = &rendered.table;
    if table.columns.is_empty() {
        return Err(RenderError::Unsupported {
            visual: rendered.id.clone(),
            reason: "table has no columns to draw".to_string(),
        });
    }
    let shown = table.rows.len().min(TABLE_MAX_ROWS);
    let cell_text = |row: &[Cell], i: usize| label(row.get(i));

    // Column widths from the longest header/value among the drawn rows.
    let widths: Vec<f64> = table
        .columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let longest = table.rows[..shown]
                .iter()
                .map(|row| cell_text(row, i).chars().count())
                .chain(std::iter::once(column.name.chars().count()))
                .max()
                .unwrap_or(0);
            (longest as f64 * TABLE_CHAR_W + 2.0 * TABLE_PAD).clamp(TABLE_MIN_COL, TABLE_MAX_COL)
        })
        .collect();

    let mut notes: Vec<String> = rendered.notes.clone();
    if table.rows.len() > shown {
        notes.push(format!(
            "Showing the first {shown} of {} rows.",
            table.rows.len()
        ));
    }

    let margin = 24.0;
    let title_h = 40.0;
    let grid_w: f64 = widths.iter().sum();
    let width = (grid_w + 2.0 * margin).max(320.0);
    let grid_h = TABLE_ROW_H * (shown as f64 + 1.0);
    let height = title_h + margin + grid_h + 16.0 + notes.len() as f64 * 16.0 + 8.0;

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"Helvetica, Arial, sans-serif\">\n"
    ));
    out.push_str(&format!(
        "<rect x=\"0\" y=\"0\" width=\"{width:.0}\" height=\"{height:.0}\" fill=\"#ffffff\"/>\n"
    ));
    out.push_str(&format!(
        "<text x=\"{margin}\" y=\"32\" font-size=\"18\" font-weight=\"bold\" fill=\"#1f2933\">{}</text>\n",
        escape_html(&rendered.title)
    ));

    let top = title_h + 8.0;
    // Header band.
    out.push_str(&format!(
        "<rect x=\"{margin}\" y=\"{top:.2}\" width=\"{grid_w:.2}\" height=\"{TABLE_ROW_H}\" fill=\"#e4e7eb\"/>\n"
    ));
    let mut x = margin;
    for (i, column) in table.columns.iter().enumerate() {
        let right = column.kind == crate::Kind::Quantitative;
        let (tx, anchor) = if right {
            (x + widths[i] - TABLE_PAD, "end")
        } else {
            (x + TABLE_PAD, "start")
        };
        let max_chars = fit_chars(widths[i]);
        out.push_str(&format!(
            "<text x=\"{tx:.2}\" y=\"{:.2}\" font-size=\"13\" font-weight=\"bold\" text-anchor=\"{anchor}\" fill=\"#1f2933\">{}</text>\n",
            top + 16.0,
            escape_html(&truncate(&column.name, max_chars))
        ));
        x += widths[i];
    }
    // Body rows.
    for (r, row) in table.rows[..shown].iter().enumerate() {
        let y = top + TABLE_ROW_H * (r as f64 + 1.0);
        if r % 2 == 1 {
            out.push_str(&format!(
                "<rect x=\"{margin}\" y=\"{y:.2}\" width=\"{grid_w:.2}\" height=\"{TABLE_ROW_H}\" fill=\"#f5f7fa\"/>\n"
            ));
        }
        let mut x = margin;
        for (i, column) in table.columns.iter().enumerate() {
            let right = column.kind == crate::Kind::Quantitative;
            let (tx, anchor) = if right {
                (x + widths[i] - TABLE_PAD, "end")
            } else {
                (x + TABLE_PAD, "start")
            };
            let max_chars = fit_chars(widths[i]);
            out.push_str(&format!(
                "<text x=\"{tx:.2}\" y=\"{:.2}\" font-size=\"12\" text-anchor=\"{anchor}\" fill=\"#1f2933\">{}</text>\n",
                y + 16.0,
                escape_html(&truncate(&cell_text(row, i), max_chars))
            ));
            x += widths[i];
        }
    }
    // Outline and bottom rule.
    out.push_str(&format!(
        "<rect x=\"{margin}\" y=\"{top:.2}\" width=\"{grid_w:.2}\" height=\"{grid_h:.2}\" fill=\"none\" stroke=\"#9aa5b1\"/>\n"
    ));
    for (i, note) in notes.iter().enumerate() {
        out.push_str(&format!(
            "<text x=\"{margin}\" y=\"{:.2}\" font-size=\"11\" fill=\"#616e7c\">{}</text>\n",
            top + grid_h + 18.0 + i as f64 * 16.0,
            escape_html(note)
        ));
    }
    out.push_str("</svg>\n");
    Ok(out)
}

struct Plot {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

struct Point {
    x: f64,
    y: f64,
    series: Option<usize>,
    size: Option<f64>,
}

fn get(row: &[Cell], index: Option<usize>) -> Option<&Cell> {
    index.and_then(|i| row.get(i))
}

fn number(cell: Option<&Cell>) -> Option<f64> {
    match cell {
        Some(Cell::Number(n)) => Some(*n),
        Some(Cell::Time { serial, .. }) => Some(*serial),
        _ => None,
    }
}

fn label(cell: Option<&Cell>) -> String {
    match cell {
        None | Some(Cell::Null) => "(null)".to_string(),
        Some(Cell::Number(n)) => fmt_num(*n),
        Some(Cell::Bool(b)) => b.to_string(),
        Some(Cell::Text(s)) => s.clone(),
        Some(Cell::Time { text, .. }) => text.clone(),
    }
}

/// Min and max of the values, widened to include zero when `include_zero`.
/// Empty input gives `(0, 1)`.
fn domain(values: impl Iterator<Item = f64>, include_zero: bool) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for v in values {
        lo = lo.min(v);
        hi = hi.max(v);
    }
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if include_zero {
        lo = lo.min(0.0);
        hi = hi.max(0.0);
    }
    (lo, hi)
}

/// "Nice" tick positions covering `[lo, hi]` with about `target` intervals.
/// Returns the padded domain start and end and the tick values.
fn nice_ticks(lo: f64, hi: f64, target: usize) -> (f64, f64, Vec<f64>) {
    let (lo, hi) = if (hi - lo).abs() < 1e-12 {
        (lo - 1.0, hi + 1.0)
    } else {
        (lo, hi)
    };
    let raw = (hi - lo) / target.max(1) as f64;
    let magnitude = 10f64.powf(raw.log10().floor());
    let normalized = raw / magnitude;
    let factor = if normalized < 1.5 {
        1.0
    } else if normalized < 3.0 {
        2.0
    } else if normalized < 7.0 {
        5.0
    } else {
        10.0
    };
    let step = magnitude * factor;
    let start = (lo / step).floor() * step;
    let end = (hi / step).ceil() * step;
    let count = ((end - start) / step).round() as usize;
    let ticks = (0..=count).map(|i| start + i as f64 * step).collect();
    (start, end, ticks)
}

/// Formats a number compactly: no trailing zeros, at most six decimals.
fn fmt_num(v: f64) -> String {
    if v == 0.0 {
        return "0".to_string();
    }
    let text = format!("{v:.6}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}
