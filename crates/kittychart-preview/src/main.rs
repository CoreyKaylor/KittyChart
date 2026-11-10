//! `kc-preview`: render KittyChart output as it would appear in a terminal, to PNG.
//!
//! For each fixture and theme it runs the same session a user would see. A shell prompt and the
//! command come first. Then capability detection is answered by Ghostty's VT core, the exact bytes
//! the `kittychart` CLI would print are written, and a fresh prompt follows. The resulting
//! terminal state (cells, colors, cursor, kitty image placements) is painted to an image. All
//! renders are combined into one contact sheet, for reviewing look and feel across terminal themes.

mod theme;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use clap::Parser;
use kittychart::ChartConfig;
use kittychart::terminal::{TermGeometry, TermOptions, render_for_terminal};
use kittychart_canvas::{Canvas, Color, Font, FontContext, Paint, SkiaCanvas, TextAlign, TextBaseline};
use kittychart_term::detect::Detector;
use libghostty_vt::kitty::graphics::PlacementIterator;
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::style::{Palette, RgbColor};
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, PrimaryDeviceAttributes,
    SecondaryDeviceAttributes, SizeReportSize, TertiaryDeviceAttributes,
};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};
use theme::Theme;

#[derive(Debug, Parser)]
#[command(about = "Screenshot KittyChart output inside a simulated terminal")]
struct Args {
    /// Chart.js config files.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Comma-separated themes: built-in names or Ghostty theme files.
    #[arg(long, value_delimiter = ',', default_values_t = theme::BUILTIN.iter().map(|s| s.to_string()))]
    theme: Vec<String>,
    /// Terminal width in cells.
    #[arg(long, default_value_t = 100)]
    cols: u16,
    /// Cell size in device pixels, WxH (Ghostty with a ~11pt font on a 2x display ≈ 16x34).
    #[arg(long, default_value = "16x34")]
    cell: String,
    /// Extra args, as given to the CLI: chart width in cells.
    #[arg(long)]
    chart_cols: Option<u16>,
    /// Output contact sheet.
    #[arg(long, default_value = "target/preview/sheet.png")]
    sheet: PathBuf,
    /// Use Chart.js's default colors instead of terminal-derived ones (what the CLI's
    /// `--colors chartjs` does).
    #[arg(long)]
    chartjs_colors: bool,
    /// Downscale the sheet by this factor (keeps it small enough to review).
    #[arg(long, default_value_t = 2.0)]
    shrink: f32,
}

const FONT_STACK: &str = "'JetBrainsMono Nerd Font', 'JetBrains Mono', 'DejaVu Sans Mono', 'Noto Sans Mono', 'Liberation Mono', monospace";

fn main() {
    let args = Args::parse();
    let (cw, ch) = args
        .cell
        .split_once('x')
        .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        .expect("--cell must look like 16x34");
    let themes: Vec<Theme> =
        args.theme.iter().map(|t| theme::resolve(t).unwrap_or_else(|e| panic!("{e}"))).collect();
    let fonts = Arc::new(Mutex::new(FontContext::with_system_fonts()));

    let mut shots = Vec::new();
    for file in &args.files {
        let json = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let config = ChartConfig::from_json(&json).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let name = file.file_name().map_or("chart".into(), |n| n.to_string_lossy().into_owned());
        for th in &themes {
            let shot =
                session(&config, &name, th, args.cols, cw, ch, args.chart_cols, args.chartjs_colors, &fonts);
            shots.push((format!("{name} · {}", th.name), shot));
        }
    }
    let sheet = contact_sheet(&shots, themes.len(), args.shrink, &fonts);
    std::fs::create_dir_all(args.sheet.parent().unwrap_or(std::path::Path::new("."))).expect("mkdir");
    kittychart_testkit::golden::write_png(
        &args.sheet,
        kittychart_testkit::golden::Image { width: sheet.0, height: sheet.1, rgba: &sheet.2 },
    );
    println!("{}", args.sheet.display());
}

type Shot = (u32, u32, Vec<u8>);

fn rgb(c: [u8; 3]) -> RgbColor {
    RgbColor { r: c[0], g: c[1], b: c[2] }
}

#[allow(clippy::too_many_arguments)]
fn session(
    config: &ChartConfig,
    file_name: &str,
    th: &Theme,
    cols: u16,
    cw: u32,
    ch: u32,
    chart_cols: Option<u16>,
    chartjs_colors: bool,
    fonts: &Arc<Mutex<FontContext>>,
) -> Shot {
    // First lay out the chart to learn how many rows the session needs.
    let geom = TermGeometry { cols, cell_width: cw as f32, cell_height: ch as f32 };
    let opts = TermOptions { cols: chart_cols, ..TermOptions::default() };
    let rows = render_for_terminal(config.clone(), geom, &opts).rows + 3;

    let mut t = Terminal::new(TerminalOptions { cols, rows, max_scrollback: 0 }).expect("terminal");
    t.resize(cols, rows, cw, ch).expect("resize");
    t.set_kitty_image_storage_limit(256 * 1024 * 1024).expect("kitty graphics");
    t.set_default_bg_color(Some(rgb(th.background))).expect("bg");
    t.set_default_fg_color(Some(rgb(th.foreground))).expect("fg");
    t.set_default_cursor_color(Some(rgb(th.cursor))).expect("cursor");
    let mut pal = t.default_color_palette().expect("palette").0;
    for (i, c) in th.palette.iter().enumerate() {
        pal[i] = rgb(*c);
    }
    t.set_default_color_palette(Some(Palette(pal))).expect("palette");

    // Capability detection, as the CLI does it, answered by the terminal.
    let replies = Rc::new(RefCell::new(Vec::new()));
    let r = replies.clone();
    t.on_pty_write(move |_, d| r.borrow_mut().extend_from_slice(d)).expect("pty");
    t.on_device_attributes(|_| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[DeviceAttributeFeature::ANSI_COLOR],
            ),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT220,
                firmware_version: 1,
                rom_cartridge: 0,
            },
            tertiary: TertiaryDeviceAttributes { unit_id: 0 },
        })
    })
    .expect("da");
    t.on_size(move |_| Some(SizeReportSize { rows, columns: cols, cell_width: cw, cell_height: ch }))
        .expect("size");

    let prompt = format!("\x1b[34m~/projects/demo\x1b[0m \x1b[32m❯\x1b[0m kittychart {file_name}\r\n");
    t.vt_write(prompt.as_bytes());
    let mut d = Detector::new();
    t.vt_write(&d.queries());
    let caps = d.feed(&replies.borrow()).cloned().unwrap_or_default();
    assert!(caps.kitty_graphics, "simulated terminal should report kitty graphics");
    let (cw2, ch2) = caps.cell_px.map_or((cw as f32, ch as f32), |(w, h)| (f32::from(w), f32::from(h)));
    let geom = TermGeometry { cols, cell_width: cw2, cell_height: ch2 };
    let colors = (!chartjs_colors)
        .then(|| {
            Some(kittychart::TerminalColors {
                background: caps.background?,
                foreground: caps.foreground?,
                palette: caps.full_palette(),
            })
        })
        .flatten();
    if !chartjs_colors {
        assert!(colors.is_some(), "terminal didn't answer OSC 10/11: {caps:?}");
    }
    if let Some(c) = &colors {
        let t = kittychart::Theme::from_terminal(c);
        let hex = |c: [u8; 3]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
        eprintln!(
            "theme {:<16} {:?} dark={} series={} surface={} ink={}/{} grid={}",
            th.name,
            t.source,
            t.dark,
            t.series.iter().map(|c| hex(*c)).collect::<Vec<_>>().join(","),
            hex(t.surface),
            hex(t.ink_primary),
            hex(t.ink_secondary),
            hex(t.grid)
        );
    }
    let opts = TermOptions { colors, ..opts };
    let out = render_for_terminal(config.clone(), geom, &opts);
    // The tty's ONLCR turns "\n" into "\r\n".
    let bytes: Vec<u8> =
        out.bytes.iter().flat_map(|&b| if b == b'\n' { vec![b'\r', b'\n'] } else { vec![b] }).collect();
    t.vt_write(&bytes);
    t.vt_write("\x1b[34m~/projects/demo\x1b[0m \x1b[32m❯\x1b[0m ".as_bytes());

    paint(&t, th, cols, rows, cw, ch, fonts)
}

fn paint(
    t: &Terminal<'_, '_>,
    th: &Theme,
    cols: u16,
    rows: u16,
    cw: u32,
    ch: u32,
    fonts: &Arc<Mutex<FontContext>>,
) -> Shot {
    let pad = cw as f32;
    let w = cols as f32 * cw as f32 + 2.0 * pad;
    let h = rows as f32 * ch as f32 + 2.0 * pad;
    let mut c = SkiaCanvas::new(w, h, 1.0).with_fonts(fonts.clone());
    let col = |c: RgbColor| Color::rgba8(c.r, c.g, c.b, 255);
    c.fill_background(Color::rgba8(th.background[0], th.background[1], th.background[2], 255));

    let mut rs = RenderState::new().expect("render state");
    let snap = rs.update(t).expect("snapshot");
    let colors = snap.colors().expect("colors");
    let font_px = ch as f32 / 1.32;
    let mut row_it_owner = RowIterator::new().expect("rows");
    let mut cell_it_owner = CellIterator::new().expect("cells");
    let mut row_iter = row_it_owner.update(&snap).expect("row iter");
    let mut y = 0u32;
    c.set_text_align(TextAlign::Left);
    c.set_text_baseline(TextBaseline::Middle);
    while let Some(row) = row_iter.next() {
        let mut cells = cell_it_owner.update(row).expect("cell iter");
        let mut x = 0u32;
        while let Some(cell) = cells.next() {
            let (px, py) = (pad + (x * cw) as f32, pad + (y * ch) as f32);
            if let Ok(Some(bg)) = cell.bg_color() {
                c.set_fill_style(Paint::Solid(col(bg)));
                c.fill_rect(px, py, cw as f32, ch as f32);
            }
            let g = cell.graphemes().unwrap_or_default();
            if !g.is_empty() {
                let style = cell.style().ok();
                let fg = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                c.set_font(Font {
                    family: FONT_STACK.into(),
                    size: font_px,
                    weight: if style.is_some_and(|s| s.bold) { 700 } else { 400 },
                    italic: false,
                });
                c.set_fill_style(Paint::Solid(col(fg)));
                let s: String = g.into_iter().collect();
                c.fill_text(&s, px, py + ch as f32 / 2.0, None);
            }
            x += 1;
        }
        y += 1;
    }
    if snap.cursor_visible().unwrap_or(false)
        && let Ok(Some(cur)) = snap.cursor_viewport()
    {
        let cc = colors.cursor.unwrap_or(colors.foreground);
        c.set_fill_style(Paint::Solid(col(cc)));
        c.fill_rect(
            pad + (u32::from(cur.x) * cw) as f32,
            pad + (u32::from(cur.y) * ch) as f32,
            cw as f32,
            ch as f32,
        );
    }

    // kitty images, above text (z ≥ 0), where the terminal placed them.
    let g = t.kitty_graphics().expect("graphics");
    let mut it = PlacementIterator::new().expect("placements");
    let mut ps = it.update(&g).expect("placement iter");
    while let Some(p) = ps.next() {
        let Some(img) = g.image(p.image_id().expect("id")) else { continue };
        let Ok(Some(pos)) = p.viewport_pos(&img, t) else { continue };
        let size = p.pixel_size(&img, t).expect("size");
        let data = img.data().expect("data").to_vec();
        c.draw_rgba_device(
            pad + pos.col as f32 * cw as f32,
            pad + pos.row as f32 * ch as f32,
            size.width as f32,
            size.height as f32,
            &data,
            img.width().expect("w"),
            img.height().expect("h"),
        );
    }
    (c.pixel_width(), c.pixel_height(), c.to_rgba())
}

fn contact_sheet(
    shots: &[(String, Shot)],
    per_row: usize,
    shrink: f32,
    fonts: &Arc<Mutex<FontContext>>,
) -> Shot {
    let gap = 24.0;
    let label_h = 28.0;
    let per_row = per_row.clamp(1, 3);
    let cell_w = shots.iter().map(|s| s.1.0).max().unwrap_or(1) as f32;
    let cell_h = shots.iter().map(|s| s.1.1).max().unwrap_or(1) as f32;
    let n_rows = shots.len().div_ceil(per_row);
    let w = per_row as f32 * (cell_w + gap) + gap;
    let h = n_rows as f32 * (cell_h + gap + label_h) + gap;
    let mut c = SkiaCanvas::new(w / shrink, h / shrink, 1.0).with_fonts(fonts.clone());
    c.fill_background(Color::rgba8(0x3a, 0x3a, 0x3a, 255));
    c.set_font(Font { size: 20.0 / shrink, weight: 700, ..Font::default() });
    c.set_text_baseline(TextBaseline::Middle);
    for (i, (label, (sw, sh, data))) in shots.iter().enumerate() {
        let (cx, cy) = (
            gap + (i % per_row) as f32 * (cell_w + gap),
            gap + (i / per_row) as f32 * (cell_h + gap + label_h),
        );
        c.set_fill_style(Paint::Solid(Color::rgba8(0xee, 0xee, 0xee, 255)));
        c.fill_text(label, cx / shrink, (cy + label_h / 2.0) / shrink, None);
        c.draw_rgba_device(
            cx / shrink,
            (cy + label_h) / shrink,
            *sw as f32 / shrink,
            *sh as f32 / shrink,
            data,
            *sw,
            *sh,
        );
    }
    (c.pixel_width(), c.pixel_height(), c.to_rgba())
}
