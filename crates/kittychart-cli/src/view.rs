//! `kittychart view FILE [--watch]`: an interactive, full-screen chart.

use std::path::PathBuf;

use clap::Parser as _;
use kittychart::ChartConfig;
use kittychart::interact::{InteractOptions, interact};
use kittychart::show::Mode;

/// Explore a chart interactively: hover for tooltips, click legend items to toggle datasets,
/// arrows to step through the data. With --watch, the chart animates to the file's new
/// contents whenever it changes.
#[derive(Debug, clap::Parser)]
#[command(name = "kittychart view", version)]
pub struct ViewArgs {
    /// Chart.js config JSON file.
    file: PathBuf,

    /// Re-read the file when it changes and animate to the new data.
    #[arg(long)]
    watch: bool,

    /// How to draw: auto, kitty, sixel, text, blocks, half or braille.
    #[arg(long, value_parser = super::PROTOCOLS)]
    protocol: Option<String>,

    /// Device pixels per CSS pixel (default: chart text about as tall as terminal text).
    #[arg(long)]
    scale: Option<f32>,

    /// Background color (any CSS color).
    #[arg(long)]
    background: Option<String>,

    /// Use Chart.js's default colors instead of the terminal's.
    #[arg(long, default_value = "terminal", value_parser = ["terminal", "chartjs"])]
    colors: String,

    /// kitty: send every frame inline (base64) even when the terminal can read temp files or
    /// shared memory.
    #[arg(long)]
    no_fast_transfer: bool,
}

pub fn run(argv: Vec<String>) -> Result<(), String> {
    let args = ViewArgs::try_parse_from(argv).map_err(|e| e.to_string())?;
    let read = |p: &std::path::Path| -> Result<ChartConfig, String> {
        let json = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        ChartConfig::from_json(&json).map_err(|e| format!("{}: {e}", p.display()))
    };
    let config = read(&args.file)?;
    let opts = InteractOptions {
        mode: args.protocol.as_deref().map(str::parse::<Mode>).transpose()?,
        scale: args.scale,
        background: match &args.background {
            Some(b) => Some(kittychart::parse_color(b).ok_or_else(|| format!("invalid color {b:?}"))?),
            None => None,
        },
        chartjs_colors: args.colors == "chartjs",
        no_fast_transfer: args.no_fast_transfer,
        watch: args.watch.then(|| args.file.clone()),
        save_dir: None,
    };
    interact(config, &opts, |json| ChartConfig::from_json(json).map_err(|e| e.to_string()))
        .map_err(|e| e.to_string())
}
