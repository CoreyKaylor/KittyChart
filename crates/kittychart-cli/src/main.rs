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

    /// Skip terminal capability detection and send kitty graphics anyway.
    #[arg(long)]
    force: bool,

    /// Default colors: `terminal` derives them from your terminal theme; `chartjs` uses
    /// Chart.js's defaults (made for white pages).
    #[arg(long, default_value = "terminal", value_parser = ["terminal", "chartjs"])]
    colors: String,
}

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
    use kittychart::terminal::{TermGeometry, TermOptions, render_for_terminal};
    let tty =
        kittychart_term::tty::Tty::open().map_err(|e| format!("no terminal ({e}); use --out FILE.png"))?;
    let ws = tty.window_size().map_err(|e| e.to_string())?;
    let caps = if args.force {
        None
    } else {
        let c = tty
            .query(Duration::from_millis(250))
            .map_err(|e| format!("couldn't query the terminal ({e}); use --force to draw anyway"))?;
        if !c.kitty_graphics {
            return Err("this terminal doesn't support the kitty graphics protocol (sixel and text \
                        fallbacks are coming). Use --out FILE.png, or --force to try anyway."
                .into());
        }
        Some(c)
    };
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
        colors: (args.colors == "terminal")
            .then(|| {
                let c = caps.as_ref()?;
                Some(kittychart::TerminalColors {
                    background: c.background?,
                    foreground: c.foreground?,
                    palette: c.full_palette(),
                })
            })
            .flatten(),
    };
    let out = render_for_terminal(config, geom, &opts).bytes;
    let mut stdout = std::io::stdout().lock();
    if stdout.is_terminal() {
        stdout.write_all(&out).and_then(|()| stdout.flush()).map_err(|e| e.to_string())
    } else {
        // stdout is redirected: draw on the terminal itself.
        let mut tty = tty;
        tty.write_all(&out).map_err(|e| e.to_string())
    }
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
