//! The native half of the `kittychart` npm package. It takes configurations as JSON strings;
//! `index.js` builds them from JavaScript values (typed arrays, Dates) and wraps them in the
//! `Chart` class.

use kittychart::show::{self, Mode, ShowOptions, TerminalInfo};
use kittychart::{CellMode, ChartConfig, RenderOptions, TerminalColors};
use napi::bindgen_prelude::*;
use napi_derive::napi;

fn invalid(msg: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, msg.into())
}

fn config(json: &str) -> Result<ChartConfig> {
    ChartConfig::from_json(json).map_err(|e| invalid(e.to_string()))
}

fn color(s: Option<&str>) -> Result<Option<kittychart::Color>> {
    s.map(|b| kittychart::parse_color(b).ok_or_else(|| invalid(format!("invalid color {b:?}")))).transpose()
}

fn positive(name: &str, v: Option<f64>) -> Result<Option<f32>> {
    match v {
        Some(x) if !(x.is_finite() && x > 0.0) => Err(invalid(format!("{name} must be a positive number"))),
        v => Ok(v.map(|x| x as f32)),
    }
}

fn cells(name: &str, v: Option<u32>) -> Result<Option<u16>> {
    v.map(|c| {
        u16::try_from(c).ok().filter(|&c| c > 0).ok_or_else(|| invalid(format!("{name} must be 1..=65535")))
    })
    .transpose()
}

fn mode_name(m: Mode) -> &'static str {
    match m {
        Mode::Auto => "auto",
        Mode::Kitty => "kitty",
        Mode::Sixel => "sixel",
        Mode::Text(CellMode::Auto) => "text",
        Mode::Text(CellMode::Blocks) => "blocks",
        Mode::Text(CellMode::HalfBlocks) => "half",
        Mode::Text(CellMode::Braille) => "braille",
    }
}

/// Check a configuration; throws with the reason if it's invalid.
#[napi]
pub fn validate(json: String) -> Result<()> {
    config(&json).map(drop)
}

#[napi(object)]
#[derive(Debug, Default)]
pub struct RenderOpts {
    /// Logical (CSS) width in pixels. Default 800.
    pub width: Option<f64>,
    /// Logical height. Default: from `options.aspectRatio`.
    pub height: Option<f64>,
    /// Device pixel ratio. Default 1.
    pub scale: Option<f64>,
    /// Any CSS color. Default: transparent.
    pub background: Option<String>,
}

fn render_options(o: &RenderOpts) -> Result<RenderOptions> {
    let mut r = RenderOptions::new(positive("width", o.width)?.unwrap_or(800.0))
        .dpr(positive("scale", o.scale)?.unwrap_or(1.0));
    r.height = positive("height", o.height)?;
    r.background = color(o.background.as_deref())?;
    Ok(r)
}

/// Render to PNG bytes.
#[napi]
pub fn render_png(json: String, opts: Option<RenderOpts>) -> Result<Buffer> {
    let r = render_options(&opts.unwrap_or_default())?;
    Ok(kittychart::render(config(&json)?, &r).to_png().into())
}

#[napi(object)]
#[allow(missing_debug_implementations)] // napi's Buffer has no Debug
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8, row by row.
    pub data: Buffer,
}

/// Render to raw RGBA pixels.
#[napi]
pub fn render_rgba(json: String, opts: Option<RenderOpts>) -> Result<RgbaImage> {
    let r = render_options(&opts.unwrap_or_default())?;
    let img = kittychart::render(config(&json)?, &r);
    Ok(RgbaImage { width: img.width, height: img.height, data: img.rgba.into() })
}

/// The terminal's colors, as `[r, g, b]` arrays.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct Colors {
    pub background: Vec<u8>,
    pub foreground: Vec<u8>,
    /// ANSI colors 0–15, when the terminal reported all of them.
    pub palette: Option<Vec<Vec<u8>>>,
}

fn rgb(v: &[u8], what: &str) -> Result<[u8; 3]> {
    <[u8; 3]>::try_from(v).map_err(|_| invalid(format!("{what} must be [r, g, b]")))
}

fn to_terminal_colors(c: &Colors) -> Result<TerminalColors> {
    let palette = match &c.palette {
        Some(p) if p.len() == 16 => {
            let mut out = [[0u8; 3]; 16];
            for (o, v) in out.iter_mut().zip(p) {
                *o = rgb(v, "palette entries")?;
            }
            Some(out)
        }
        _ => None,
    };
    Ok(TerminalColors {
        background: rgb(&c.background, "background")?,
        foreground: rgb(&c.foreground, "foreground")?,
        palette,
    })
}

fn from_terminal_colors(c: &TerminalColors) -> Colors {
    Colors {
        background: c.background.to_vec(),
        foreground: c.foreground.to_vec(),
        palette: c.palette.map(|p| p.iter().map(|c| c.to_vec()).collect()),
    }
}

/// Render as text cells: block and Braille characters with truecolor ANSI escapes. `mode` is
/// "text" (default), "blocks", "half" or "braille".
#[napi]
pub fn render_text(
    json: String,
    cols: u32,
    rows: Option<u32>,
    mode: Option<String>,
    colors: Option<Colors>,
) -> Result<String> {
    let cell = match mode.as_deref().unwrap_or("text") {
        "text" => CellMode::Auto,
        "blocks" => CellMode::Blocks,
        "half" => CellMode::HalfBlocks,
        "braille" => CellMode::Braille,
        m => return Err(invalid(format!("unknown text mode {m:?}; expected text, blocks, half or braille"))),
    };
    let cols = cells("cols", Some(cols))?.unwrap_or(80);
    let rows = cells("rows", rows)?;
    let colors = colors.as_ref().map(to_terminal_colors).transpose()?;
    let r =
        kittychart::terminal::render_cells_for_terminal(config(&json)?, cols, rows, cell, colors.as_ref());
    String::from_utf8(r.bytes).map_err(|e| Error::from_reason(e.to_string()))
}

/// What the terminal is and supports.
#[napi(object)]
#[derive(Debug, Clone)]
pub struct TermInfo {
    pub cols: u32,
    pub rows: u32,
    /// Cell size in device pixels.
    pub cell_width: f64,
    pub cell_height: f64,
    pub kitty: bool,
    pub sixel: bool,
    pub tmux: bool,
    pub colors: Option<Colors>,
}

/// Ask the controlling terminal for its size, graphics support and colors. `null` when there
/// is no terminal.
#[napi]
pub fn probe(timeout_ms: Option<u32>) -> Option<TermInfo> {
    let t = std::time::Duration::from_millis(u64::from(timeout_ms.unwrap_or(250)));
    show::probe(t).ok().map(|i| TermInfo {
        cols: u32::from(i.cols),
        rows: u32::from(i.rows),
        cell_width: f64::from(i.cell_width),
        cell_height: f64::from(i.cell_height),
        kitty: i.kitty,
        sixel: i.sixel,
        tmux: i.tmux,
        colors: i.colors.as_ref().map(from_terminal_colors),
    })
}

#[napi(object)]
#[derive(Debug, Default)]
pub struct ShowOpts {
    /// "auto", "kitty", "sixel", "text", "blocks", "half" or "braille". Default:
    /// `$KITTYCHART_PROTOCOL`, else "auto".
    pub protocol: Option<String>,
    /// Width in cells. Default: the terminal width.
    pub cols: Option<u32>,
    /// Height in cells. Default: from the chart's aspect ratio.
    pub rows: Option<u32>,
    /// Logical width/height in CSS pixels (override cols/rows).
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Device pixels per CSS pixel. Default: chart text about as tall as terminal text.
    pub scale: Option<f64>,
    pub background: Option<String>,
    /// kitty: draw with Unicode placeholders (automatic inside tmux).
    pub placeholders: Option<bool>,
    /// Skip terminal detection.
    pub force: Option<bool>,
    /// "terminal" (default): colors follow the terminal theme; "chartjs": Chart.js defaults.
    pub colors: Option<String>,
    /// kitty image id. Default: a fresh one.
    pub image_id: Option<u32>,
}

fn show_options(o: &ShowOpts) -> Result<ShowOptions> {
    let chartjs_colors = match o.colors.as_deref() {
        None | Some("terminal") => false,
        Some("chartjs") => true,
        Some(c) => return Err(invalid(format!("colors must be \"terminal\" or \"chartjs\", not {c:?}"))),
    };
    Ok(ShowOptions {
        mode: o.protocol.as_deref().map(str::parse::<Mode>).transpose().map_err(invalid)?,
        cols: cells("cols", o.cols)?,
        rows: cells("rows", o.rows)?,
        width: positive("width", o.width)?,
        height: positive("height", o.height)?,
        scale: positive("scale", o.scale)?,
        background: color(o.background.as_deref())?,
        placeholders: o.placeholders.unwrap_or(false),
        force: o.force.unwrap_or(false),
        chartjs_colors,
        image_id: o.image_id.map(|i| i.clamp(1, 0x00FF_FFFF)),
    })
}

/// A chart ready to write to the terminal.
#[napi(object)]
#[allow(missing_debug_implementations)] // napi's Buffer has no Debug
pub struct Prepared {
    /// "kitty", "sixel", "text", "blocks", "half" or "braille".
    pub mode: String,
    /// Escapes or ANSI text, ending with a newline.
    pub data: Buffer,
    /// "stdout", or "tty" when stdout is redirected (graphics go to the terminal itself).
    pub target: String,
    pub cols: u32,
    pub rows: u32,
    /// `data.subarray(0, graphicsLen)` is the graphics escapes alone.
    pub graphics_len: u32,
    pub image_id: Option<u32>,
}

fn to_js(p: show::Prepared) -> Prepared {
    Prepared {
        mode: mode_name(p.mode).into(),
        target: match p.target {
            show::Target::Stdout => "stdout",
            show::Target::Tty => "tty",
        }
        .into(),
        cols: u32::from(p.cols),
        rows: u32::from(p.rows),
        graphics_len: p.graphics_len as u32,
        image_id: p.image_id,
        data: p.bytes.into(),
    }
}

/// Detect the terminal, choose a mode and render, without writing.
#[napi]
pub fn prepare(json: String, opts: Option<ShowOpts>) -> Result<Prepared> {
    let o = show_options(&opts.unwrap_or_default())?;
    let p = show::prepare(config(&json)?, &o).map_err(|e| Error::from_reason(e.to_string()))?;
    Ok(to_js(p))
}

/// Render for a known terminal (from `probe`) without any I/O.
#[napi]
pub fn prepare_for(json: String, info: TermInfo, opts: Option<ShowOpts>) -> Result<Prepared> {
    let o = show_options(&opts.unwrap_or_default())?;
    let info = TerminalInfo {
        cols: u16::try_from(info.cols).unwrap_or(u16::MAX),
        rows: u16::try_from(info.rows).unwrap_or(u16::MAX),
        cell_width: info.cell_width as f32,
        cell_height: info.cell_height as f32,
        kitty: info.kitty,
        sixel: info.sixel,
        tmux: info.tmux,
        colors: info.colors.as_ref().map(to_terminal_colors).transpose()?,
    };
    let m = o.mode.unwrap_or(Mode::Auto);
    Ok(to_js(show::prepare_for(config(&json)?, &info, m, &o)))
}

/// Write bytes to the controlling terminal (`/dev/tty`). Unix only.
#[napi]
pub fn write_tty(data: Buffer) -> Result<()> {
    #[cfg(unix)]
    {
        let mut tty = kittychart::term::tty::Tty::open().map_err(|e| Error::from_reason(e.to_string()))?;
        tty.write_all(&data).map_err(|e| Error::from_reason(e.to_string()))
    }
    #[cfg(not(unix))]
    {
        let _ = data;
        Err(Error::from_reason("no Unix terminal"))
    }
}

/// The library version.
#[napi]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
