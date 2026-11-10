//! `kittychart`: render a Chart.js config in the terminal (kitty graphics protocol) or to PNG.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use kittychart::{ChartConfig, RenderOptions};

/// Render a Chart.js configuration (JSON) as a chart in your terminal, or to a PNG file.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Chart.js config JSON file. Reads stdin when omitted or `-`.
    file: Option<PathBuf>,

    /// Write a PNG (or, for a `.svg` name, an SVG) here instead of displaying in the terminal.
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
        let bytes = if out.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg")) {
            kittychart::render_svg(config, &opts).into_bytes()
        } else {
            kittychart::render(config, &opts).to_png()
        };
        std::fs::write(out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
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
    use kittychart::show::{Mode, ShowOptions, show};

    let mode = match &args.protocol {
        Some(p) => Some(p.parse::<Mode>()?),
        None => None,
    };
    let opts = ShowOptions {
        mode,
        cols: args.cols,
        rows: args.rows,
        width: args.width,
        height: args.height,
        scale: args.scale,
        background,
        placeholders: args.placeholders,
        force: args.force,
        chartjs_colors: args.colors == "chartjs",
        image_id: None,
    };
    show(config, &opts).map(drop).map_err(|e| e.to_string())
}

fn read_stdin() -> Result<String, String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}
