//! Drawing a chart in the user's terminal: detect what it supports, pick kitty graphics, sixel
//! or text, size the chart to the window and write it at the cursor.
//!
//! This is the one place in the facade that does I/O (through `kittychart_term::tty`). The
//! CLI and every language binding go through it, so they all detect and fall back the same way.
//!
//! Detection needs a Unix terminal or the Windows console (Windows is unverified so far).
//! Elsewhere [`prepare`] draws text on stdout and [`probe`] returns `Unsupported`;
//! [`prepare_for`] works everywhere.
//!
//! ```no_run
//! let config = kittychart::ChartConfig::from_json(r#"{"type":"bar","data":{"labels":["a"],"datasets":[{"data":[1]}]}}"#)?;
//! kittychart::show::show(config, &kittychart::show::ShowOptions::default())?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
#[cfg(any(unix, windows))]
use std::io::IsTerminal;
use std::io::{self, Write};
use std::str::FromStr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use kittychart_canvas::cells::CellMode;
#[cfg(any(unix, windows))]
use kittychart_term::detect::Capabilities;
#[cfg(any(unix, windows))]
use kittychart_term::tty::{self, Tty};

use crate::terminal::{Protocol, TermGeometry, TermOptions, render_cells_for_terminal, render_for_terminal};
use crate::{ChartConfig, Color, TerminalColors};

/// How to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// kitty graphics if the terminal supports them, else sixel, else text.
    #[default]
    Auto,
    Kitty,
    Sixel,
    /// Block and Braille characters.
    Text(CellMode),
}

impl Mode {
    /// The names accepted by [`Mode::from_str`] (and the CLI's `--protocol`).
    pub const NAMES: [&'static str; 7] = ["auto", "kitty", "sixel", "text", "blocks", "half", "braille"];

    /// `$KITTYCHART_PROTOCOL`, if set and non-empty.
    pub fn from_env() -> Result<Option<Mode>, ShowError> {
        match std::env::var("KITTYCHART_PROTOCOL") {
            Ok(p) if !p.is_empty() => p.parse().map(Some).map_err(|_| {
                ShowError::Usage(format!("KITTYCHART_PROTOCOL={p:?} is not one of {:?}", Mode::NAMES))
            }),
            _ => Ok(None),
        }
    }
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(s: &str) -> Result<Mode, String> {
        Ok(match s {
            "auto" => Mode::Auto,
            "kitty" => Mode::Kitty,
            "sixel" => Mode::Sixel,
            "text" => Mode::Text(CellMode::Auto),
            "blocks" => Mode::Text(CellMode::Blocks),
            "half" => Mode::Text(CellMode::HalfBlocks),
            "braille" => Mode::Text(CellMode::Braille),
            _ => return Err(format!("unknown protocol {s:?}; expected one of {:?}", Mode::NAMES)),
        })
    }
}

/// Options for [`show`]. Every field is optional.
#[derive(Debug, Clone, Default)]
pub struct ShowOptions {
    /// `None`: `$KITTYCHART_PROTOCOL`, else [`Mode::Auto`].
    pub mode: Option<Mode>,
    /// Width in cells (default: the terminal width).
    pub cols: Option<u16>,
    /// Height in cells (default: from `options.aspectRatio`).
    pub rows: Option<u16>,
    /// Logical width in CSS px (overrides `cols`).
    pub width: Option<f32>,
    /// Logical height in CSS px (overrides `rows`).
    pub height: Option<f32>,
    /// Device pixels per CSS pixel (default: chart text about as tall as terminal text).
    pub scale: Option<f32>,
    pub background: Option<Color>,
    /// kitty: draw with Unicode placeholders (automatic inside tmux).
    pub placeholders: bool,
    /// Skip detection. With [`Mode::Auto`] this sends kitty graphics.
    pub force: bool,
    /// Use Chart.js's own default colors instead of deriving them from the terminal theme.
    pub chartjs_colors: bool,
    /// kitty image id. Default: a fresh id per call, so earlier charts stay on screen.
    pub image_id: Option<u32>,
}

/// What went wrong.
#[derive(Debug)]
pub enum ShowError {
    /// The request can't work on this terminal (wrong protocol, no terminal …). The message
    /// says what to do instead.
    Usage(String),
    Io(io::Error),
}

impl fmt::Display for ShowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShowError::Usage(m) => f.write_str(m),
            ShowError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ShowError {}

impl From<io::Error> for ShowError {
    fn from(e: io::Error) -> Self {
        ShowError::Io(e)
    }
}

/// What the terminal is and supports. From [`probe`], or built by hand for a known terminal.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalInfo {
    pub cols: u16,
    pub rows: u16,
    /// Cell size in device pixels (10×20 when the terminal doesn't say).
    pub cell_width: f32,
    pub cell_height: f32,
    pub kitty: bool,
    pub sixel: bool,
    /// Inside tmux: kitty graphics need passthrough and placeholders.
    pub tmux: bool,
    /// The terminal's colors, when it reported them.
    pub colors: Option<TerminalColors>,
}

impl TerminalInfo {
    #[cfg(any(unix, windows))]
    fn new(ws: tty::WindowSize, caps: Option<&Capabilities>) -> TerminalInfo {
        let (cell_width, cell_height) = ws
            .cell_px()
            .or_else(|| caps.and_then(|c| c.cell_px).filter(|(w, h)| *w > 0 && *h > 0))
            .map_or((10.0, 20.0), |(w, h)| (f32::from(w), f32::from(h)));
        let colors = caps.and_then(|c| {
            Some(TerminalColors {
                background: c.background?,
                foreground: c.foreground?,
                palette: c.full_palette(),
            })
        });
        TerminalInfo {
            cols: ws.cols,
            rows: ws.rows,
            cell_width,
            cell_height,
            kitty: caps.is_some_and(|c| c.kitty_graphics),
            sixel: caps.is_some_and(|c| c.sixel),
            tmux: tty::in_tmux(),
            colors,
        }
    }

    /// The best mode this terminal supports.
    pub fn best_mode(&self) -> Mode {
        if self.kitty {
            Mode::Kitty
        } else if self.sixel {
            Mode::Sixel
        } else {
            Mode::Text(CellMode::Auto)
        }
    }
}

/// How long [`probe`] waits for the terminal's replies.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// Ask the controlling terminal (`/dev/tty`) for its size, graphics support and colors.
/// Unsupported outside Unix and Windows.
#[cfg(not(any(unix, windows)))]
pub fn probe(_timeout: Duration) -> io::Result<TerminalInfo> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "terminal detection needs a Unix or Windows terminal"))
}

/// Ask the controlling terminal (`/dev/tty`) for its size, graphics support and colors.
#[cfg(any(unix, windows))]
pub fn probe(timeout: Duration) -> io::Result<TerminalInfo> {
    let tty = Tty::open()?;
    let ws = tty.window_size()?;
    let caps = tty.query(timeout)?;
    Ok(TerminalInfo::new(ws, Some(&caps)))
}

/// Where the bytes of a [`Prepared`] chart should go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// Standard output: a terminal, or (text mode only) a pipe or file.
    Stdout,
    /// The controlling terminal, because stdout is redirected.
    Tty,
}

/// A chart ready to write.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub mode: Mode,
    /// Escapes or ANSI text, ending with a newline that leaves the cursor below the chart.
    pub bytes: Vec<u8>,
    pub target: Target,
    pub cols: u16,
    pub rows: u16,
    /// `bytes[..graphics_len]` are the graphics escapes alone (0 in text mode). With kitty
    /// placeholders the rest is the placeholder text, for hosts that lay out text themselves.
    pub graphics_len: usize,
    /// The kitty image id (kitty mode only).
    pub image_id: Option<u32>,
}

impl Prepared {
    /// Write to [`Prepared::target`].
    pub fn write(&self) -> io::Result<()> {
        match self.target {
            Target::Stdout => {
                let mut out = io::stdout().lock();
                out.write_all(&self.bytes)?;
                out.flush()
            }
            #[cfg(any(unix, windows))]
            Target::Tty => Tty::open()?.write_all(&self.bytes),
            #[cfg(not(any(unix, windows)))]
            Target::Tty => Err(io::Error::new(io::ErrorKind::Unsupported, "no /dev/tty")),
        }
    }
}

/// Detect, choose a mode and render, without writing anything.
pub fn prepare(config: ChartConfig, opts: &ShowOptions) -> Result<Prepared, ShowError> {
    let mode = match opts.mode {
        Some(m) => m,
        None => Mode::from_env()?.unwrap_or_default(),
    };
    #[cfg(not(any(unix, windows)))]
    return match mode {
        Mode::Auto | Mode::Text(_) => Ok(text_on_stdout(config, mode, opts)),
        _ => Err(ShowError::Usage("graphics need a Unix or Windows terminal; render a PNG instead".into())),
    };
    #[cfg(any(unix, windows))]
    prepare_tty(config, mode, opts)
}

/// No terminal to ask (a pipe, cron, CI, or another platform): text, on stdout, `$COLUMNS` wide.
fn text_on_stdout(config: ChartConfig, mode: Mode, opts: &ShowOptions) -> Prepared {
    let cell = if let Mode::Text(c) = mode { c } else { CellMode::Auto };
    let cols = opts.cols.or_else(env_columns).unwrap_or(80).max(10);
    let r = render_cells_for_terminal(config, cols, opts.rows, cell, None);
    Prepared {
        mode: Mode::Text(cell),
        bytes: r.bytes,
        target: Target::Stdout,
        cols: r.grid.cols as u16,
        rows: r.grid.rows as u16,
        graphics_len: 0,
        image_id: None,
    }
}

#[cfg(any(unix, windows))]
fn prepare_tty(config: ChartConfig, mode: Mode, opts: &ShowOptions) -> Result<Prepared, ShowError> {
    let tty = match Tty::open() {
        Ok(t) => t,
        Err(_) if matches!(mode, Mode::Auto | Mode::Text(_)) => {
            return Ok(text_on_stdout(config, mode, opts));
        }
        Err(e) => return Err(ShowError::Usage(format!("no terminal ({e}); render a PNG instead"))),
    };
    let ws = tty.window_size()?;
    let caps = if opts.force {
        None
    } else {
        match tty.query(PROBE_TIMEOUT) {
            Ok(c) => Some(c),
            // Graphics need a terminal that answers; text works anywhere.
            Err(_) if mode == Mode::Auto => None,
            Err(e) => {
                return Err(ShowError::Usage(format!(
                    "couldn't query the terminal ({e}); force drawing to skip the query"
                )));
            }
        }
    };
    let mut info = TerminalInfo::new(ws, caps.as_ref());
    let mode = match (mode, &caps) {
        (Mode::Auto, None) if opts.force => Mode::Kitty,
        (Mode::Auto, Some(_)) => info.best_mode(),
        (Mode::Auto, None) => Mode::Text(CellMode::Auto),
        (Mode::Kitty, Some(c)) if !c.kitty_graphics => {
            return Err(ShowError::Usage(
                "this terminal doesn't support the kitty graphics protocol (inside tmux, \
                 `set -g allow-passthrough on`). Use protocol auto to fall back, or force to send it anyway."
                    .into(),
            ));
        }
        (Mode::Sixel, Some(c)) if !c.sixel => {
            return Err(ShowError::Usage(
                "this terminal doesn't report sixel support. Use protocol auto to fall back, or force to send it anyway."
                    .into(),
            ));
        }
        (m, _) => m,
    };
    if opts.chartjs_colors {
        info.colors = None;
    }
    let mut p = prepare_for(config, &info, mode, opts);
    if !io::stdout().is_terminal() {
        p.target = Target::Tty; // stdout is redirected: draw on the terminal itself
    }
    Ok(p)
}

/// Sans-IO: render for a known terminal in a given mode (never [`Mode::Auto`]; that resolves to
/// [`TerminalInfo::best_mode`]). Hosts that detect once and draw many charts (Rich, Textual,
/// IPython) use this with the result of [`probe`].
pub fn prepare_for(config: ChartConfig, info: &TerminalInfo, mode: Mode, opts: &ShowOptions) -> Prepared {
    let mode = if mode == Mode::Auto { info.best_mode() } else { mode };
    match mode {
        Mode::Kitty | Mode::Sixel | Mode::Auto => {
            let image_id = opts.image_id.unwrap_or_else(next_image_id);
            let geom = TermGeometry {
                cols: info.cols.max(20),
                cell_width: info.cell_width,
                cell_height: info.cell_height,
            };
            let topts = TermOptions {
                cols: opts.cols,
                rows: opts.rows,
                width: opts.width,
                height: opts.height,
                scale: opts.scale,
                background: opts.background,
                image_id: Some(image_id),
                colors: info.colors.clone(),
                protocol: if mode == Mode::Sixel { Protocol::Sixel } else { Protocol::Kitty },
                placeholders: opts.placeholders,
                tmux: info.tmux,
            };
            let r = render_for_terminal(config, geom, &topts);
            Prepared {
                mode,
                bytes: r.bytes,
                target: Target::Stdout,
                cols: r.cols,
                rows: r.rows,
                graphics_len: r.graphics_len,
                image_id: (mode != Mode::Sixel).then_some(image_id),
            }
        }
        Mode::Text(cell) => {
            let cols = opts.cols.unwrap_or(info.cols).max(10);
            let r = render_cells_for_terminal(config, cols, opts.rows, cell, info.colors.as_ref());
            Prepared {
                mode,
                bytes: r.bytes,
                target: Target::Stdout,
                cols: r.grid.cols as u16,
                rows: r.grid.rows as u16,
                graphics_len: 0,
                image_id: None,
            }
        }
    }
}

/// Detect, render and write: draw `config` at the cursor and leave the cursor below it.
pub fn show(config: ChartConfig, opts: &ShowOptions) -> Result<Prepared, ShowError> {
    let p = prepare(config, opts)?;
    p.write()?;
    Ok(p)
}

fn env_columns() -> Option<u16> {
    std::env::var("COLUMNS").ok()?.parse().ok().filter(|&c| c > 0)
}

/// A fresh kitty image id: unique per call within a process, and spread by process id so
/// separate processes (or separate CLI runs) don't replace each other's images.
pub fn next_image_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let base = std::process::id().wrapping_mul(2_654_435_761);
    (base.wrapping_add(n.wrapping_mul(97)) & 0x00FF_FFFF).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> TerminalInfo {
        TerminalInfo {
            cols: 80,
            rows: 24,
            cell_width: 10.0,
            cell_height: 20.0,
            kitty: true,
            sixel: true,
            tmux: false,
            colors: None,
        }
    }

    fn config() -> ChartConfig {
        ChartConfig::from_json(r#"{"type":"bar","data":{"labels":["a"],"datasets":[{"data":[1]}]}}"#).unwrap()
    }

    #[test]
    fn modes_parse() {
        for name in Mode::NAMES {
            assert!(name.parse::<Mode>().is_ok(), "{name}");
        }
        assert!("png".parse::<Mode>().is_err());
    }

    #[test]
    fn auto_prefers_kitty_then_sixel_then_text() {
        let mut i = info();
        assert_eq!(i.best_mode(), Mode::Kitty);
        i.kitty = false;
        assert_eq!(i.best_mode(), Mode::Sixel);
        i.sixel = false;
        assert_eq!(i.best_mode(), Mode::Text(CellMode::Auto));
    }

    #[test]
    fn prepare_for_each_mode() {
        let opts = ShowOptions { cols: Some(40), ..Default::default() };
        let k = prepare_for(config(), &info(), Mode::Auto, &opts);
        assert_eq!(k.mode, Mode::Kitty);
        assert!(k.bytes.starts_with(b"\x1b_G"));
        assert_eq!(k.cols, 40);
        let s = prepare_for(config(), &info(), Mode::Sixel, &opts);
        assert!(s.bytes.starts_with(b"\x1bP"));
        let t = prepare_for(config(), &info(), Mode::Text(CellMode::Auto), &opts);
        assert!(t.bytes.ends_with(b"\r\n") && t.cols == 40);
    }

    #[test]
    fn image_ids_are_fresh_and_valid() {
        let ids: Vec<u32> = (0..1000).map(|_| next_image_id()).collect();
        assert!(ids.iter().all(|&i| (1..=0x00FF_FFFF).contains(&i)));
        let mut d = ids.clone();
        d.sort_unstable();
        d.dedup();
        assert_eq!(d.len(), ids.len());
    }
}
