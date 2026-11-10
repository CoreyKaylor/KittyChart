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

#[cfg(any(unix, windows))]
pub mod interact;
pub mod session;
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
    /// Elements drawn in their hover state (Chart.js `setActiveElements`).
    pub active: Vec<kittychart_core::interaction::ActiveElement>,
    /// Show a tooltip for `active` (a pinned tooltip), with this pointer position for the
    /// `nearest` positioner.
    pub tooltip: Option<(f64, f64)>,
    /// Datasets hidden (as by legend clicks).
    pub hidden: Vec<usize>,
    /// Pie, doughnut and polar area data indices hidden (as by legend clicks).
    pub hidden_data: Vec<usize>,
    /// Render the intro animation's frame this many milliseconds in (one-shot renders
    /// otherwise draw the final state).
    pub time: Option<f64>,
    /// With `time`: first finish the intro, then update to this `data` and render `time` ms
    /// into that transition (for testing update animations).
    #[doc(hidden)]
    pub next_data: Option<serde_json::Value>,
}

impl RenderOptions {
    pub fn new(width: f32) -> Self {
        Self {
            width,
            height: None,
            dpr: 1.0,
            background: None,
            pixel_size: None,
            theme: None,
            active: Vec::new(),
            tooltip: None,
            hidden: Vec::new(),
            hidden_data: Vec::new(),
            time: None,
            next_data: None,
        }
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

    /// Options from a test fixture's `"_kc"` object: `width` (512), `height`, `dpr` (1),
    /// `background` (white), `active` (`[[dataset, index], …]`), `tooltip`
    /// (`{"position": {"x", "y"}}`), `hidden` and `hiddenData`. Used by the golden, parity,
    /// SVG and gallery tools, so they all render a fixture the same way.
    #[doc(hidden)]
    pub fn from_fixture(kc: &serde_json::Value) -> RenderOptions {
        use serde_json::Value;
        let num = |k: &str, d: f64| kc.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
        let mut opts = RenderOptions::new(num("width", 512.0)).dpr(num("dpr", 1.0));
        opts.height = kc.get("height").and_then(Value::as_f64).map(|h| h as f32);
        opts.background = parse_color(kc.get("background").and_then(Value::as_str).unwrap_or("#ffffff"));
        let list = |k: &str| -> Vec<usize> {
            kc.get(k)
                .and_then(Value::as_array)
                .map_or(vec![], |a| a.iter().filter_map(Value::as_u64).map(|v| v as usize).collect())
        };
        opts.hidden = list("hidden");
        opts.hidden_data = list("hiddenData");
        if let Some(active) = kc.get("active").and_then(Value::as_array) {
            opts.active = active
                .iter()
                .filter_map(|a| {
                    Some(kittychart_core::interaction::ActiveElement {
                        dataset: a.get(0)?.as_u64()? as usize,
                        index: a.get(1)?.as_u64()? as usize,
                    })
                })
                .collect();
        }
        if let Some(t) = kc.get("tooltip") {
            let n = |k: &str| t["position"].get(k).and_then(Value::as_f64).unwrap_or(0.0);
            opts.tooltip = Some((n("x"), n("y")));
        }
        opts.time = kc.get("time").and_then(Value::as_f64);
        opts.next_data = kc.get("next").cloned();
        opts
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

/// A chart set up for `opts`: theme, visibility, hover and tooltip state.
fn chart_for(config: ChartConfig, opts: &RenderOptions) -> Chart {
    let mut chart = Chart::with_theme(config, opts.theme.clone());
    for &d in &opts.hidden {
        chart.set_dataset_visibility(d, false);
    }
    for &i in &opts.hidden_data {
        chart.toggle_data_visibility(i);
    }
    chart.set_active(opts.active.clone());
    if let Some(event) = opts.tooltip {
        chart.set_tooltip(opts.active.clone(), Some(event));
    }
    chart.dpr = f64::from(opts.dpr);
    chart
}

/// With `opts.time`: run the chart's clock so the next draw is that frame of the intro (or,
/// with `next_data`, of the update to it). Frames before it draw to a scratch canvas.
fn play(chart: &mut Chart, opts: &RenderOptions, px_w: u32, px_h: u32) {
    let Some(t) = opts.time else { return };
    let scratch = || SkiaCanvas::with_pixel_size(px_w.max(1), px_h.max(1), opts.dpr);
    // The intro starts at the first frame; render the one `t` ms later.
    chart.set_time(0.0);
    chart.draw(&mut scratch());
    match &opts.next_data {
        Some(next) => {
            chart.set_time(5000.0);
            chart.draw(&mut scratch());
            let mut config = chart.config.clone();
            config.data = next.clone();
            chart.update(config);
            chart.draw(&mut scratch());
            chart.set_time(5000.0 + t);
        }
        None => chart.set_time(t),
    }
}

/// Render a configuration.
pub fn render(config: ChartConfig, opts: &RenderOptions) -> Image {
    let mut chart = chart_for(config, opts);
    let height = opts.height.unwrap_or_else(|| (f64::from(opts.width) / chart.aspect_ratio()) as f32);
    let mut canvas = match opts.pixel_size {
        Some((w, h)) => SkiaCanvas::with_pixel_size(w, h, opts.dpr),
        None => SkiaCanvas::new(opts.width, height, opts.dpr),
    };
    if let Some(bg) = opts.background {
        canvas.fill_background(bg);
    }
    play(&mut chart, opts, canvas.pixel_width(), canvas.pixel_height());
    chart.draw(&mut canvas);
    Image { width: canvas.pixel_width(), height: canvas.pixel_height(), rgba: canvas.to_rgba() }
}

/// Render as an SVG document, the same chart as [`render`] (`pixel_size` is ignored). Text is
/// drawn as glyph outlines, so the SVG needs no fonts.
pub fn render_svg(config: ChartConfig, opts: &RenderOptions) -> String {
    let mut chart = chart_for(config, opts);
    let height = opts.height.unwrap_or_else(|| (f64::from(opts.width) / chart.aspect_ratio()) as f32);
    let mut canvas = kittychart_canvas::SvgCanvas::new(opts.width, height, opts.dpr);
    if let Some(bg) = opts.background {
        canvas.fill_background(bg);
    }
    let px = |v: f32| (v * opts.dpr).ceil() as u32;
    play(&mut chart, opts, px(opts.width), px(height));
    chart.dpr = f64::from(opts.dpr);
    chart.set_active(opts.active.clone());
    if let Some(event) = opts.tooltip {
        chart.set_tooltip(opts.active.clone(), Some(event));
    }
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
