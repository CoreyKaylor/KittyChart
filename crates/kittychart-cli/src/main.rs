//! `kittychart`: render a Chart.js config in the terminal (kitty graphics protocol) or to PNG.

use std::io::Read;
#[cfg(unix)]
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;
#[cfg(unix)]
use std::time::Duration;

use clap::Parser;
use kittychart::{ChartConfig, RenderOptions};

/// Render a Chart.js configuration (JSON) as a chart in your terminal, or to a PNG file.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Chart.js config JSON file. Reads stdin when omitted or `-`.
    file: Option<PathBuf>,

    /// Write a PNG here instead of displaying in the terminal.
    #[arg(short, long)]
    out: Option<PathBuf>,

    /// Width in terminal cells (default: the full terminal width).
    #[arg(long)]
    cols: Option<u16>,

    /// Height in terminal cells (default: from `options.aspectRatio`).
    #[arg(long)]
    rows: Option<u16>,

    /// Logical chart width in CSS pixels (PNG output, or to override the terminal size).
    #[arg(short, long)]
    width: Option<f32>,

    /// Logical chart height in CSS pixels.
    #[arg(short = 'H', long)]
    height: Option<f32>,

    /// Device pixels per CSS pixel. In a terminal the default makes chart text about as large
    /// as terminal text. For PNG output the default is 1.
    #[arg(long)]
    scale: Option<f32>,

    /// Background color (any CSS color). Default: transparent.
    #[arg(long)]
    background: Option<String>,

    /// How to draw: `auto` picks kitty graphics, then sixel, then text. `text` draws with
    /// block and Braille characters; `blocks`, `half` and `braille` force one character set.
    /// Default: `$KITTYCHART_PROTOCOL`, else `auto`.
    #[arg(long, value_parser = PROTOCOLS)]
    protocol: Option<String>,

    /// kitty: draw with Unicode placeholders, so the image lives in the text grid (automatic
    /// inside tmux).
    #[arg(long)]
    placeholders: bool,

    /// Skip terminal capability detection (with `auto`, send kitty graphics anyway).
    #[arg(long)]
    force: bool,

    /// Default colors: `terminal` derives them from your terminal theme; `chartjs` uses
    /// Chart.js's defaults (made for white pages).
    #[arg(long, default_value = "terminal", value_parser = ["terminal", "chartjs"])]
    colors: String,
}

const PROTOCOLS: [&str; 7] = ["auto", "kitty", "sixel", "text", "blocks", "half", "braille"];

fn main() -> ExitCode {
    match run(Args::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kittychart: {e}");
            ExitCode::FAILURE
        }
    }
}

fn positive(name: &str, v: Option<f32>) -> Result<(), String> {
    match v {
        Some(x) if !(x.is_finite() && x > 0.0) => Err(format!("--{name} must be a positive number")),
        _ => Ok(()),
    }
}

fn run(args: Args) -> Result<(), String> {
    positive("width", args.width)?;
    positive("height", args.height)?;
    positive("scale", args.scale)?;
    if args.cols == Some(0) || args.rows == Some(0) {
        return Err("--cols and --rows must be at least 1".into());
    }
    let json = match args.file.as_deref() {
        None => read_stdin()?,
        Some(p) if p.as_os_str() == "-" => read_stdin()?,
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?,
    };
    let config = ChartConfig::from_json(&json).map_err(|e| e.to_string())?;
    let background = match &args.background {
        Some(b) => Some(kittychart_core::color::parse(b).ok_or_else(|| format!("invalid color {b:?}"))?),
        None => None,
    };

    if let Some(out) = &args.out {
        let mut opts = RenderOptions::new(args.width.unwrap_or(800.0)).dpr(args.scale.unwrap_or(1.0));
        opts.height = args.height;
        opts.background = background;
        let img = kittychart::render(config, &opts);
        std::fs::write(out, img.to_png()).map_err(|e| format!("{}: {e}", out.display()))?;
        return Ok(());
    }
    display(config, &args, background)
}

#[cfg(not(unix))]
fn display(_config: ChartConfig, _args: &Args, _background: Option<kittychart::Color>) -> Result<(), String> {
    Err("drawing in the terminal is only supported on Unix for now; use --out FILE.png".into())
}

#[cfg(unix)]
fn display(config: ChartConfig, args: &Args, background: Option<kittychart::Color>) -> Result<(), String> {
    use kittychart::terminal::{
        Protocol, TermGeometry, TermOptions, render_cells_for_terminal, render_for_terminal,
    };

    let env = std::env::var("KITTYCHART_PROTOCOL").ok().filter(|p| !p.is_empty());
    let protocol = match (&args.protocol, env) {
        (Some(p), _) => p.clone(),
        (None, Some(p)) if PROTOCOLS.contains(&p.as_str()) => p,
        (None, Some(p)) => return Err(format!("KITTYCHART_PROTOCOL={p:?} is not one of {PROTOCOLS:?}")),
        (None, None) => "auto".into(),
    };
    let tmux = kittychart_term::tty::in_tmux();

    let text_mode = ["text", "blocks", "half", "braille"].contains(&protocol.as_str());
    let tty = match kittychart_term::tty::Tty::open() {
        Ok(t) => t,
        // No terminal at all (a pipe, cron, CI): text still works, on stdout.
        Err(_) if protocol == "auto" || text_mode => {
            let mode = cell_mode(&protocol);
            let cols = args.cols.or_else(env_columns).unwrap_or(80).max(10);
            let out = render_cells_for_terminal(config, cols, args.rows, mode, None).bytes;
            let mut stdout = std::io::stdout().lock();
            return stdout.write_all(&out).and_then(|()| stdout.flush()).map_err(|e| e.to_string());
        }
        Err(e) => return Err(format!("no terminal ({e}); use --out FILE.png")),
    };
    let ws = tty.window_size().map_err(|e| e.to_string())?;
    let caps = if args.force {
        None
    } else {
        match tty.query(Duration::from_millis(250)) {
            Ok(c) => Some(c),
            // Graphics need a terminal that answers; text works anywhere.
            Err(_) if protocol == "auto" => None,
            Err(e) => {
                return Err(format!("couldn't query the terminal ({e}); use --force to draw anyway"));
            }
        }
    };
    let protocol = match (protocol.as_str(), &caps) {
        ("auto", None) if args.force => "kitty",
        ("auto", Some(c)) if c.kitty_graphics => "kitty",
        ("auto", Some(c)) if c.sixel => "sixel",
        ("auto", _) => "text",
        ("kitty", Some(c)) if !c.kitty_graphics => {
            return Err("this terminal doesn't support the kitty graphics protocol (inside tmux, \
                        `set -g allow-passthrough on`). Try --protocol auto, or --force."
                .into());
        }
        ("sixel", Some(c)) if !c.sixel => {
            return Err("this terminal doesn't report sixel support. Try --protocol auto, or --force.".into());
        }
        (p, _) => p,
    };
    let colors = (args.colors == "terminal")
        .then(|| {
            let c = caps.as_ref()?;
            Some(kittychart::TerminalColors {
                background: c.background?,
                foreground: c.foreground?,
                palette: c.full_palette(),
            })
        })
        .flatten();

    let out = match protocol {
        "kitty" | "sixel" => {
            let (cell_width, cell_height) = ws
                .cell_px()
                .or_else(|| caps.as_ref().and_then(|c| c.cell_px).filter(|(w, h)| *w > 0 && *h > 0))
                .map_or((10.0, 20.0), |(w, h)| (f32::from(w), f32::from(h)));
            let geom = TermGeometry { cols: ws.cols.max(20), cell_width, cell_height };
            let opts = TermOptions {
                cols: args.cols,
                rows: args.rows,
                width: args.width,
                height: args.height,
                scale: args.scale,
                background,
                image_id: Some(image_id()),
                colors,
                protocol: if protocol == "sixel" { Protocol::Sixel } else { Protocol::Kitty },
                placeholders: args.placeholders,
                tmux,
            };
            render_for_terminal(config, geom, &opts).bytes
        }
        cells => {
            let mode = cell_mode(cells);
            let cols = args.cols.unwrap_or(ws.cols).max(10);
            render_cells_for_terminal(config, cols, args.rows, mode, colors.as_ref()).bytes
        }
    };
    let mut stdout = std::io::stdout().lock();
    if stdout.is_terminal() {
        stdout.write_all(&out).and_then(|()| stdout.flush()).map_err(|e| e.to_string())
    } else {
        // stdout is redirected: draw on the terminal itself.
        let mut tty = tty;
        tty.write_all(&out).map_err(|e| e.to_string())
    }
}

#[cfg(unix)]
fn cell_mode(protocol: &str) -> kittychart_canvas::cells::CellMode {
    use kittychart_canvas::cells::CellMode;
    match protocol {
        "blocks" => CellMode::Blocks,
        "half" => CellMode::HalfBlocks,
        "braille" => CellMode::Braille,
        _ => CellMode::Auto,
    }
}

#[cfg(unix)]
fn env_columns() -> Option<u16> {
    std::env::var("COLUMNS").ok()?.parse().ok().filter(|&c| c > 0)
}

/// A per-process image id, so separate invocations don't replace each other's images.
#[cfg(unix)]
fn image_id() -> u32 {
    (std::process::id() % 0x00FF_FFFF).max(1)
}

fn read_stdin() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}
