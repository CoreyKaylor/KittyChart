//! The docs gallery: every fixture in `tests/fixtures`, rendered twice, on a white page with
//! Chart.js's own colors and on a dark terminal (Tokyo Night) with colors derived from the
//! terminal's theme. Writes `docs/gallery/*.png` and `docs/gallery.md`.
//!
//! Run with `just gallery`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kittychart::{ChartConfig, RenderOptions, TerminalColors, Theme, render};
use kittychart_testkit::golden::{Image, write_png};

const WIDTH: f32 = 480.0;
const DPR: f32 = 1.5;

fn tokyo_night() -> TerminalColors {
    let hex = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
    TerminalColors {
        background: hex(0x1a1b26),
        foreground: hex(0xa9b1d6),
        palette: Some(
            [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868,
                0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ]
            .map(hex),
        ),
    }
}

fn main() {
    let root = kittychart_testkit::workspace_root();
    let fixtures = root.join("tests/fixtures");
    let docs = root.join("docs");
    let out = docs.join("gallery");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let dark = tokyo_night();

    let mut md = String::from(
        "# Gallery\n\n\
         Every chart in `tests/fixtures/`, each a plain Chart.js configuration. Left: Chart.js's \
         default colors on a white page. Right: the same config in a dark terminal (Tokyo Night), \
         with default colors derived from the terminal's theme.\n\n\
         Regenerate with `just gallery`.\n",
    );
    let mut section = String::new();
    for file in walk(&fixtures) {
        let rel = file.strip_prefix(&fixtures).unwrap();
        let group = rel.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let name = rel.with_extension("").to_string_lossy().replace('/', "-");
        let json = std::fs::read_to_string(&file).unwrap();
        let config = ChartConfig::from_json(&json).unwrap_or_else(|e| panic!("{}: {e}", file.display()));

        // The fixture's state (hover, tooltip, hidden data) at the gallery's size.
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut light = RenderOptions::from_fixture(&v["_kc"]);
        light.width = WIDTH;
        light.dpr = DPR;
        light.height = None;
        // Hover and tooltip positions are in fixture pixels; keep them at the fixture's width.
        if light.tooltip.is_some() || !light.active.is_empty() {
            light.width = v["_kc"]["width"].as_f64().unwrap_or(512.0) as f32;
            light.height = v["_kc"]["height"].as_f64().map(|h| h as f32);
        }
        light.background = kittychart::parse_color("#ffffff");
        let mut night = light.clone();
        night.background = kittychart::parse_color("#1a1b26");
        night.theme = Some(Theme::from_terminal(&dark));
        for (suffix, opts) in [("light", &light), ("dark", &night)] {
            let img = render(config.clone(), opts);
            write_png(
                &out.join(format!("{name}-{suffix}.png")),
                Image { width: img.width, height: img.height, rgba: &img.rgba },
            );
        }

        if group != section {
            let _ = write!(md, "\n## {group}\n");
            section = group;
        }
        let src = format!("../tests/fixtures/{}", rel.to_string_lossy());
        let _ = write!(
            md,
            "\n### [{name}]({src})\n\n<img src=\"gallery/{name}-light.png\" width=\"400\"> \
             <img src=\"gallery/{name}-dark.png\" width=\"400\">\n"
        );
    }
    std::fs::write(docs.join("gallery.md"), md).unwrap();
    println!("wrote {}", docs.join("gallery.md").display());
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|e| e == "json") {
            out.push(p);
        }
    }
    out
}
