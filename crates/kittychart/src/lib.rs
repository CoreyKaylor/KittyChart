//! KittyChart: Chart.js-compatible charts for graphics-capable terminals.
//!
//! ```no_run
//! let config = r#"{
//!   "type": "bar",
//!   "data": { "labels": ["Q1", "Q2", "Q3"], "datasets": [{ "label": "Revenue", "data": [12, 19, 7] }] }
//! }"#;
//! let image = kittychart::render_json(config, &kittychart::RenderOptions::new(800.0)).unwrap();
//! std::fs::write("chart.png", image.to_png()).unwrap();
//! ```

/// The drawing layer (Canvas2D-style API, raster and cell backends).
pub use kittychart_canvas as canvas;
pub use kittychart_canvas::cells::{CellGrid, CellMode};
pub use kittychart_canvas::{Color, FontContext};
/// Parse any CSS color (`#36a2eb`, `rgb(…)`, `hsl(…)`, named colors, `transparent`).
pub use kittychart_core::color::parse as parse_color;
/// Scriptable options: functions in place of option values (see [`script::Script`]).
pub use kittychart_core::script;
pub use kittychart_core::{Chart, ChartConfig, ChartType, ConfigError, Label, TerminalColors, Theme};

pub mod show;
pub mod terminal;

/// The terminal layer (encoders, detection, `/dev/tty`), for hosts that need lower-level access.
pub use kittychart_term as term;

use kittychart_canvas::SkiaCanvas;

/// How to render a chart.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Logical (CSS) width in pixels.
    pub width: f32,
    /// Logical height. `None` derives it from `options.aspectRatio` (default 2), as Chart.js does.
    pub height: Option<f32>,
    /// Device pixel ratio. The bitmap is `width·dpr × height·dpr`.
    pub dpr: f32,
    /// Fill behind the chart. `None` leaves it transparent, as a Chart.js canvas is.
    pub background: Option<Color>,
    /// Render at exactly this many device pixels (overriding `width`/`height`·`dpr` rounding).
    pub pixel_size: Option<(u32, u32)>,
    /// Replace Chart.js's default colors with a theme (for example one derived from the
    /// terminal). Explicit colors in the config still win.
    pub theme: Option<Theme>,
}

impl RenderOptions {
    pub fn new(width: f32) -> Self {
        Self { width, height: None, dpr: 1.0, background: None, pixel_size: None, theme: None }
    }
    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h);
        self
    }
    pub fn dpr(mut self, dpr: f32) -> Self {
        self.dpr = dpr;
        self
    }
    pub fn background(mut self, c: Color) -> Self {
        self.background = Some(c);
        self
    }
}

/// A rendered chart: non-premultiplied RGBA8.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn to_png(&self) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("in-memory PNG header");
            w.write_image_data(&self.rgba).expect("in-memory PNG data");
        }
        out
    }

    pub fn as_term_image(&self) -> kittychart_term::RgbaImage<'_> {
        kittychart_term::RgbaImage::new(self.width, self.height, &self.rgba)
    }
}

/// Render a configuration.
pub fn render(config: ChartConfig, opts: &RenderOptions) -> Image {
    let mut chart = Chart::with_theme(config, opts.theme.clone());
    let height = opts.height.unwrap_or_else(|| (f64::from(opts.width) / chart.aspect_ratio()) as f32);
    let mut canvas = match opts.pixel_size {
        Some((w, h)) => SkiaCanvas::with_pixel_size(w, h, opts.dpr),
        None => SkiaCanvas::new(opts.width, height, opts.dpr),
    };
    if let Some(bg) = opts.background {
        canvas.fill_background(bg);
    }
    chart.dpr = f64::from(opts.dpr);
    chart.draw(&mut canvas);
    Image { width: canvas.pixel_width(), height: canvas.pixel_height(), rgba: canvas.to_rgba() }
}

/// Render as an SVG document, the same chart as [`render`] (`pixel_size` is ignored). Text is
/// drawn as glyph outlines, so the SVG needs no fonts.
pub fn render_svg(config: ChartConfig, opts: &RenderOptions) -> String {
    let mut chart = Chart::with_theme(config, opts.theme.clone());
    let height = opts.height.unwrap_or_else(|| (f64::from(opts.width) / chart.aspect_ratio()) as f32);
    let mut canvas = kittychart_canvas::SvgCanvas::new(opts.width, height, opts.dpr);
    if let Some(bg) = opts.background {
        canvas.fill_background(bg);
    }
    chart.dpr = f64::from(opts.dpr);
    chart.draw(&mut canvas);
    canvas.finish()
}

/// Render as terminal cells (the text fallback): `cols × rows` cells, graphics as block or
/// Braille characters, text as real characters. `theme` should come from the terminal's
/// colors (see [`Theme::from_terminal`]); without it Chart.js's light-page colors are used.
pub fn render_cells(
    config: ChartConfig,
    cols: u32,
    rows: u32,
    mode: kittychart_canvas::cells::CellMode,
    theme: Option<Theme>,
) -> kittychart_canvas::cells::CellGrid {
    let mut chart = Chart::for_cells(config, theme);
    let mut canvas = kittychart_canvas::cells::CellCanvas::new(cols.max(1), rows.max(1), mode);
    chart.draw(&mut canvas);
    canvas.into_grid()
}

/// Parse a Chart.js JSON configuration and render it.
pub fn render_json(json: &str, opts: &RenderOptions) -> Result<Image, ConfigError> {
    Ok(render(ChartConfig::from_json(json)?, opts))
}
