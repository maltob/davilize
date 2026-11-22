//! The refresh pipeline: runs every visual of a report and writes the requested
//! outputs into a folder. Shared by `refresh`, `publish` and `serve --refresh`.

use std::path::{Path, PathBuf};

use dv_core::Report;
use dv_engine::Engine;
use serde_json::json;

use crate::{
    CliError, CliResult, DataOutput, EXIT_INPUT, Log, base_dir, file_stem_name, output_error,
    param_error, write_output,
};

/// What to write and where the HTML page goes.
#[derive(Clone)]
pub struct Options {
    pub formats: Vec<DataOutput>,
    /// Directory with vega.min.js, vega-lite.min.js and vega-embed.min.js (inline HTML).
    pub vendor_dir: Option<PathBuf>,
    /// File name for the HTML page. Defaults to `<report-file-stem>.html`.
    pub html_name: Option<String>,
    /// Parameter values as `NAME=VALUE` pairs. Defaults apply to parameters not listed.
    pub params: Vec<(String, String)>,
}

/// Summary of a finished refresh.
pub struct Output {
    pub report_name: String,
    pub report_version: u32,
}

/// Runs every visual of `report_path` and writes its outputs into `out_dir`.
pub fn run(report_path: &Path, out_dir: &Path, opts: &Options, log: &Log) -> CliResult<Output> {
    let report = Report::load(report_path)?;
    let values =
        dv_core::resolve_parameters(&report.parameters, &opts.params).map_err(param_error)?;
    let stem = file_stem_name(report_path)?;
    let engine = Engine::new()?;
    engine.register_report(&report, base_dir(report_path))?;

    std::fs::create_dir_all(out_dir)
        .map_err(|e| output_error(format!("cannot create '{}': {e}", out_dir.display())))?;
    log.info(
        &format!(
            "refreshing '{}' ({} visuals)",
            report.name,
            report.visuals.len()
        ),
        json!({"report": report.name, "visuals": report.visuals.len()}),
    );

    let formats = &opts.formats;
    // With dashboards, the HTML page is built from its own per-placement renders,
    // so the per-visual render is only needed for the other formats.
    let html_from_visuals = report.dashboards.is_empty();
    let needs_render = formats
        .iter()
        .any(|f| f.engine_format().is_none() && (*f != DataOutput::Html || html_from_visuals));
    let mut rendered = Vec::new();
    for visual in &report.visuals {
        let bound = dv_core::bind_visual(visual, &values).map_err(param_error)?;
        let sql = engine.compile(&bound)?;
        for format in formats {
            if let Some(engine_format) = format.engine_format() {
                let path = out_dir.join(format!("{}.{}", visual.id, engine_format.extension()));
                let rows = engine.export(&sql, &path, engine_format)?;
                log.info(
                    &format!("wrote {} ({rows} rows)", path.display()),
                    json!({"visual": visual.id, "path": path.display().to_string(), "rows": rows}),
                );
            }
        }
        if needs_render {
            let batches = engine.query(&sql)?;
            let item = dv_render::prepare(&bound, &batches)?;
            write_visual_graphics(&item, formats, out_dir, log)?;
            rendered.push(item);
        }
    }

    if formats.contains(&DataOutput::Html) {
        let vendored = match &opts.vendor_dir {
            Some(dir) => Some(VendoredScripts::read(dir)?),
            None => None,
        };
        let scripts = match &vendored {
            Some(v) => dv_render::Scripts::Inline {
                vega: &v.vega,
                vega_lite: &v.vega_lite,
                vega_embed: &v.vega_embed,
            },
            None => dv_render::Scripts::Cdn,
        };
        let name = opts
            .html_name
            .clone()
            .unwrap_or_else(|| format!("{stem}.html"));
        let path = out_dir.join(name);
        let path_text = path.display().to_string();
        if report.dashboards.is_empty() {
            let page = dv_render::html_report(&report.name, &rendered, &scripts)?;
            write_output(&path, page.as_bytes())?;
            log.info(
                &format!("wrote {} ({} visuals)", path.display(), rendered.len()),
                json!({"path": path_text, "visuals": rendered.len()}),
            );
        } else {
            // Each placement is queried with its dashboard's shared filters added.
            let views = dv_render::dashboard_views(
                &report,
                &values,
                |dataset| {
                    Ok(engine
                        .schema(dataset)?
                        .into_iter()
                        .map(|c| c.name)
                        .collect())
                },
                |visual| {
                    let sql = engine.compile(visual)?;
                    let batches = engine.query(&sql)?;
                    Ok(dv_render::prepare(visual, &batches)?)
                },
                param_error,
            )?;
            let page = dv_render::html_dashboards(&report.name, &views, &scripts)?;
            write_output(&path, page.as_bytes())?;
            log.info(
                &format!("wrote {} ({} dashboards)", path.display(), views.len()),
                json!({"path": path_text, "dashboards": views.len()}),
            );
        }
    }
    if formats.contains(&DataOutput::Xlsx) {
        let bytes = dv_render::xlsx::workbook(&rendered)?;
        let path = out_dir.join(format!("{stem}.xlsx"));
        write_output(&path, &bytes)?;
        log.info(
            &format!("wrote {} ({} sheets)", path.display(), rendered.len()),
            json!({"path": path.display().to_string(), "sheets": rendered.len()}),
        );
    }
    log.info(
        &format!("refresh complete: {} visuals", report.visuals.len()),
        json!({"status": "ok"}),
    );
    Ok(Output {
        report_name: report.name.clone(),
        report_version: report.version,
    })
}

/// Writes the per-visual SVG, PNG and PDF outputs the user asked for. Every
/// chart type has a native SVG renderer.
fn write_visual_graphics(
    item: &dv_render::Rendered,
    formats: &[DataOutput],
    out_dir: &Path,
    log: &Log,
) -> CliResult<()> {
    let wanted = |f: DataOutput| formats.contains(&f);
    if !(wanted(DataOutput::Svg) || wanted(DataOutput::Png) || wanted(DataOutput::Pdf)) {
        return Ok(());
    }
    let svg = dv_render::svg::render(item)?;
    if wanted(DataOutput::Svg) {
        write_visual_file(out_dir, &item.id, "svg", svg.as_bytes(), log)?;
    }
    if wanted(DataOutput::Png) {
        let png = dv_render::raster::png(&svg)?;
        write_visual_file(out_dir, &item.id, "png", &png, log)?;
    }
    if wanted(DataOutput::Pdf) {
        let pdf = dv_render::raster::pdf(&svg)?;
        write_visual_file(out_dir, &item.id, "pdf", &pdf, log)?;
    }
    Ok(())
}

fn write_visual_file(
    out_dir: &Path,
    id: &str,
    extension: &str,
    bytes: &[u8],
    log: &Log,
) -> CliResult<()> {
    let path = out_dir.join(format!("{id}.{extension}"));
    write_output(&path, bytes)?;
    log.info(
        &format!("wrote {}", path.display()),
        json!({"visual": id, "path": path.display().to_string()}),
    );
    Ok(())
}

/// The three UMD bundles read from a vendor directory for `--offline` HTML.
struct VendoredScripts {
    vega: String,
    vega_lite: String,
    vega_embed: String,
}

impl VendoredScripts {
    fn read(dir: &Path) -> CliResult<Self> {
        let read = |name: &str| -> CliResult<String> {
            let path = dir.join(name);
            std::fs::read_to_string(&path).map_err(|e| CliError {
                code: EXIT_INPUT,
                message: format!("cannot read vendored '{}': {e}", path.display()),
            })
        };
        Ok(VendoredScripts {
            vega: read("vega.min.js")?,
            vega_lite: read("vega-lite.min.js")?,
            vega_embed: read("vega-embed.min.js")?,
        })
    }
}
