//! Sizing and encoding a chart for a terminal grid. Sans-IO: callers supply the terminal
//! geometry (from a real TTY, or a simulated one in tests and previews) and get escape bytes.

use kittychart_term::kitty::{self, TransmitOptions};

use crate::{Chart, ChartConfig, Color, Image, RenderOptions, render};

/// What we know about the terminal.
#[derive(Debug, Clone, Copy)]
pub struct TermGeometry {
    pub cols: u16,
    /// Cell size in device pixels.
    pub cell_width: f32,
    pub cell_height: f32,
}

/// How to fit the chart into the terminal. Every field is optional. The bitmap always covers a
/// whole number of cells exactly, so the terminal displays it 1:1 without resampling.
#[derive(Debug, Clone, Default)]
pub struct TermOptions {
    /// Width in cells (default: the full terminal width).
    pub cols: Option<u16>,
    /// Height in cells (default: from `options.aspectRatio`).
    pub rows: Option<u16>,
    /// Logical width in CSS px. Overrides `cols`, rounded to whole cells.
    pub width: Option<f32>,
    /// Logical height in CSS px. Overrides `rows`, rounded to whole cells.
    pub height: Option<f32>,
    /// Device pixels per CSS pixel (default: chart text about as tall as terminal text).
    pub scale: Option<f32>,
    pub background: Option<Color>,
    /// kitty image id (default 1).
    pub image_id: Option<u32>,
    /// The terminal's colors. When set, chart defaults are derived from them (see
    /// [`kittychart_core::theme`]); otherwise Chart.js's defaults are used unchanged.
    pub colors: Option<crate::TerminalColors>,
}

/// A chart laid out for the terminal, with the escape sequences that display it at the cursor.
#[derive(Debug, Clone)]
pub struct TermRender {
    pub image: Image,
    pub cols: u16,
    pub rows: u16,
    pub scale: f32,
    /// kitty graphics escapes, followed by a newline that moves the cursor below the image.
    pub bytes: Vec<u8>,
}

/// The default scale: Chart.js text is 12px with 1.2 line height, so a chart text line is about
/// as tall as a terminal line.
pub fn default_scale(cell_height: f32) -> f32 {
    (cell_height / 16.0).clamp(1.0, 4.0)
}

pub fn render_for_terminal(config: ChartConfig, geom: TermGeometry, opts: &TermOptions) -> TermRender {
    let (cw, ch) = (geom.cell_width.max(1.0), geom.cell_height.max(1.0));
    let scale = opts.scale.filter(|s| *s > 0.0).unwrap_or_else(|| default_scale(ch));
    let cells = |px: f32, cell: f32| ((px / cell).round().clamp(1.0, f32::from(u16::MAX))) as u16;
    let cols = match (opts.width, opts.cols) {
        (Some(w), _) => cells(w * scale, cw),
        (None, c) => c.unwrap_or(geom.cols).max(1),
    };
    let logical_w = f32::from(cols) * cw / scale;
    let rows = match (opts.height, opts.rows) {
        (Some(h), _) => cells(h * scale, ch),
        (None, Some(r)) => r.max(1),
        (None, None) => {
            let aspect = Chart::new(config.clone()).aspect_ratio() as f32;
            cells(logical_w / aspect * scale, ch)
        }
    };
    // Exact device pixels: whole cells, no float round trip.
    let (px_w, px_h) = ((f32::from(cols) * cw).round() as u32, (f32::from(rows) * ch).round() as u32);

    let mut ropts = RenderOptions::new(logical_w).height(px_h as f32 / scale).dpr(scale);
    ropts.pixel_size = Some((px_w, px_h));
    ropts.background = opts.background;
    ropts.theme = opts.colors.as_ref().map(crate::Theme::from_terminal);
    let image = render(config, &ropts);

    let mut bytes = Vec::with_capacity(image.rgba.len() / 4);
    let mut t = TransmitOptions::new(opts.image_id.unwrap_or(1));
    t.columns = Some(u32::from(cols));
    t.rows = Some(u32::from(rows));
    kitty::transmit_and_display(image.as_term_image(), &t, &mut bytes);
    bytes.push(b'\n');
    TermRender { image, cols, rows, scale, bytes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ChartConfig {
        ChartConfig::from_json(r#"{"type":"bar","data":{"labels":["a"],"datasets":[{"data":[1]}]}}"#).unwrap()
    }

    #[test]
    fn bitmap_is_exactly_whole_cells() {
        // Cell sizes and widths that previously rounded one pixel over.
        for (cw, ch, cols) in [(29.0, 59.0, 61u16), (16.0, 34.0, 100), (7.0, 15.0, 33), (13.0, 27.0, 211)] {
            let geom = TermGeometry { cols, cell_width: cw, cell_height: ch };
            let r = render_for_terminal(config(), geom, &TermOptions::default());
            assert_eq!(r.image.width, (f32::from(r.cols) * cw) as u32, "{cw}x{ch} cols {cols}");
            assert_eq!(r.image.height, (f32::from(r.rows) * ch) as u32, "{cw}x{ch} cols {cols}");
        }
    }

    #[test]
    fn explicit_size_snaps_to_cells_and_height_overrides_rows() {
        let geom = TermGeometry { cols: 120, cell_width: 16.0, cell_height: 34.0 };
        let opts =
            TermOptions { width: Some(500.0), height: Some(160.0), rows: Some(10), ..TermOptions::default() };
        let r = render_for_terminal(config(), geom, &opts);
        assert_eq!(r.image.width, u32::from(r.cols) * 16);
        assert_eq!(r.rows, ((160.0 * r.scale) / 34.0f32).round() as u16, "height wins over rows");
    }
}
