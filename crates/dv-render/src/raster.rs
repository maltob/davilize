//! SVG to PNG (resvg) and SVG to PDF (svg2pdf). Both parse the SVG with the
//! same usvg version, and text is laid out with the system font database.

use std::sync::{Arc, OnceLock};

use resvg::{tiny_skia, usvg};

use crate::RenderError;

/// PNG output is rendered at this multiple of the SVG size, for legibility.
pub const PNG_SCALE: f32 = 2.0;

/// System fonts, loaded once per process (scanning fonts is slow).
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

fn parse(svg: &str) -> Result<usvg::Tree, RenderError> {
    let options = usvg::Options {
        fontdb: fonts(),
        ..usvg::Options::default()
    };
    usvg::Tree::from_str(svg, &options)
        .map_err(|e| RenderError::Output(format!("cannot parse SVG: {e}")))
}

/// Rasterises an SVG document to PNG bytes at [`PNG_SCALE`].
pub fn png(svg: &str) -> Result<Vec<u8>, RenderError> {
    let tree = parse(svg)?;
    let size = tree.size();
    let width = (size.width() * PNG_SCALE).ceil() as u32;
    let height = (size.height() * PNG_SCALE).ceil() as u32;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| RenderError::Output(format!("invalid image size {width}x{height}")))?;
    let transform = tiny_skia::Transform::from_scale(PNG_SCALE, PNG_SCALE);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|e| RenderError::Output(format!("cannot encode PNG: {e}")))
}

/// Converts an SVG document to a one-page PDF with vector content and text.
pub fn pdf(svg: &str) -> Result<Vec<u8>, RenderError> {
    let tree = parse(svg)?;
    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|e| RenderError::Output(format!("cannot convert SVG to PDF: {e}")))
}
