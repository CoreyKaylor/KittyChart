//! Sizing and encoding a chart for a terminal grid. Sans-IO: callers supply the terminal
//! geometry (from a real TTY, or a simulated one in tests and previews) and get escape bytes.

use kittychart_canvas::cells::{CELL_H, CELL_W, CellGrid, CellMode};
use kittychart_term::kitty::{self, TransmitOptions};
use kittychart_term::sixel::{self, SixelOptions};

use crate::{Chart, ChartConfig, Color, Image, RenderOptions, render};

/// What we know about the terminal.
#[derive(Debug, Clone, Copy)]
pub struct TermGeometry {
    pub cols: u16,
    /// Cell size in device pixels.
    pub cell_width: f32,
    pub cell_height: f32,
}

/// How the bitmap is sent to the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    /// The kitty graphics protocol (kitty, Ghostty, WezTerm, Konsole …).
    #[default]
    Kitty,
    /// Sixel (foot, xterm, mlterm, Windows Terminal, iTerm2 …).
    Sixel,
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
    pub protocol: Protocol,
    /// kitty: draw with Unicode placeholders (a virtual placement plus placeholder text), so
    /// the image lives in the text grid. Implied by `tmux`.
    pub placeholders: bool,
    /// Running inside tmux: kitty escapes are wrapped for passthrough and drawn with
    /// placeholders, so tmux keeps the image in its pane.
    pub tmux: bool,
}

/// A chart laid out for the terminal, with the escape sequences that display it at the cursor.
#[derive(Debug, Clone)]
pub struct TermRender {
    pub image: Image,
    pub cols: u16,
    pub rows: u16,
    pub scale: f32,
    /// Graphics escapes, followed by a newline that moves the cursor below the image.
    pub bytes: Vec<u8>,
    /// `bytes[..graphics_len]` are the graphics escapes alone. With placeholders, the rest is
    /// the placeholder text (for hosts that lay out text themselves, like Rich or Textual).
    pub graphics_len: usize,
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
    // Placeholder rows are numbered by diacritics, and the table has 297 of them.
    let rows =
        if opts.placeholders || opts.tmux { rows.min(kitty::MAX_PLACEHOLDER_ROWS as u16) } else { rows };
    // Exact device pixels: whole cells, no float round trip.
    let (px_w, px_h) = ((f32::from(cols) * cw).round() as u32, (f32::from(rows) * ch).round() as u32);

    let mut ropts = RenderOptions::new(logical_w).height(px_h as f32 / scale).dpr(scale);
    ropts.pixel_size = Some((px_w, px_h));
    ropts.background = opts.background;
    ropts.theme = opts.colors.as_ref().map(crate::Theme::from_terminal);
    let image = render(config, &ropts);

    let mut bytes = Vec::with_capacity(image.rgba.len() / 4);
    let mut graphics_len = usize::MAX;
    match opts.protocol {
        Protocol::Kitty => {
            let mut t = TransmitOptions::new(opts.image_id.unwrap_or(1));
            t.columns = Some(u32::from(cols));
            t.rows = Some(u32::from(rows));
            if opts.placeholders || opts.tmux {
                let mut esc = Vec::new();
                kitty::transmit_virtual(image.as_term_image(), &t, &mut esc);
                bytes = if opts.tmux { kitty::tmux_passthrough(&esc) } else { esc };
                graphics_len = bytes.len();
                let id = opts.image_id.unwrap_or(1);
                bytes.extend_from_slice(
                    kitty::placeholder_text(id, u32::from(cols), u32::from(rows)).as_bytes(),
                );
                bytes.push(b'\r');
            } else {
                kitty::transmit_and_display(image.as_term_image(), &t, &mut bytes);
            }
        }
        Protocol::Sixel => {
            let background = opts.colors.as_ref().map(|c| c.background);
            sixel::encode(
                image.as_term_image(),
                &SixelOptions { background, ..SixelOptions::default() },
                &mut bytes,
            );
        }
    }
    let graphics_len = graphics_len.min(bytes.len());
    bytes.push(b'\n');
    TermRender { image, cols, rows, scale, bytes, graphics_len }
}

/// A chart as text cells: the fallback for terminals without graphics.
#[derive(Debug, Clone)]
pub struct CellRender {
    pub grid: CellGrid,
    /// Truecolor ANSI text, ending with a newline.
    pub bytes: Vec<u8>,
}

/// Render as text cells, `cols` wide; `rows` defaults to the chart's aspect ratio (terminal
/// cells are about twice as tall as wide). With `colors`, chart defaults follow the terminal's
/// theme.
pub fn render_cells_for_terminal(
    config: ChartConfig,
    cols: u16,
    rows: Option<u16>,
    mode: CellMode,
    colors: Option<&crate::TerminalColors>,
) -> CellRender {
    let cols = cols.max(1);
    let rows = rows.unwrap_or_else(|| {
        let aspect = Chart::new(config.clone()).aspect_ratio() as f32;
        ((f32::from(cols) * CELL_W / aspect / CELL_H).round() as u16).max(4)
    });
    let grid = crate::render_cells(
        config,
        u32::from(cols),
        u32::from(rows),
        mode,
        colors.map(crate::Theme::from_terminal),
    );
    let mut bytes = grid.to_ansi().into_bytes();
    bytes.extend_from_slice(b"\r\n");
    CellRender { grid, bytes }
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

    #[test]
    fn sixel_output_decodes_to_the_image() {
        let geom = TermGeometry { cols: 30, cell_width: 8.0, cell_height: 16.0 };
        let opts = TermOptions { protocol: Protocol::Sixel, ..TermOptions::default() };
        let r = render_for_terminal(config(), geom, &opts);
        let d = kittychart_testkit::sixel::decode(&r.bytes);
        assert_eq!((d.width, d.height), (r.image.width, r.image.height));
    }

    #[test]
    fn tmux_wraps_escapes_and_draws_placeholders() {
        let geom = TermGeometry { cols: 30, cell_width: 8.0, cell_height: 16.0 };
        let opts = TermOptions { tmux: true, image_id: Some(7), ..TermOptions::default() };
        let r = render_for_terminal(config(), geom, &opts);
        assert!(r.bytes.starts_with(b"\x1bPtmux;\x1b\x1b_Ga=T,U=1,"));
        let text = String::from_utf8_lossy(&r.bytes);
        let cells = text.matches(kitty::PLACEHOLDER).count();
        assert_eq!(cells, usize::from(r.cols) * usize::from(r.rows));
        assert!(text.contains("\x1b[38;2;0;0;7m"));
        let (graphics, rest) = r.bytes.split_at(r.graphics_len);
        assert!(
            graphics.ends_with(b"\x1b\\") && !String::from_utf8_lossy(graphics).contains(kitty::PLACEHOLDER)
        );
        assert_eq!(String::from_utf8_lossy(rest).matches(kitty::PLACEHOLDER).count(), cells);
    }

    #[test]
    fn cell_rows_follow_the_aspect_ratio() {
        // Bar charts default to aspectRatio 2: 40 cols × 8 px = 320 px wide → 160 px = 10 rows.
        let r = render_cells_for_terminal(config(), 40, None, CellMode::Auto, None);
        assert_eq!((r.grid.cols, r.grid.rows), (40, 10));
        assert!(r.bytes.ends_with(b"\r\n"));
    }
}
