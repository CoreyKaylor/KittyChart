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
use kittychart::terminal::{Protocol, TermGeometry, TermOptions, render_for_terminal};
use kittychart_canvas::{Canvas, Color, Font, FontContext, Paint, SkiaCanvas, TextAlign, TextBaseline};
use kittychart_term::detect::Detector;
use kittychart_term::kitty;
use libghostty_vt::kitty::graphics::PlacementIterator;
use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::style::RgbColor;
use libghostty_vt::{RenderState, Terminal};
use theme::Theme;

#[derive(Debug, Parser)]
#[command(about = "Screenshot KittyChart output inside a simulated terminal")]
struct Args {
    /// Chart.js config files.
    #[arg(required_unless_present = "exec")]
    files: Vec<PathBuf>,
    /// Instead of simulating the CLI: run this real command (whitespace-separated argv) on a
    /// PTY attached to the simulated terminal, and screenshot the screen.
    #[arg(long)]
    exec: Option<String>,
    /// With --exec: extra environment, NAME=VALUE (repeatable).
    #[arg(long)]
    env: Vec<String>,
    /// With --exec: keys to type, "MS:TEXT,…" (escapes \r \n \e \s).
    #[arg(long, default_value = "")]
    keys: String,
    /// With --exec: screenshot at this many ms (then kill the command) instead of at exit.
    #[arg(long)]
    at: Option<u64>,
    /// With --exec: terminal height in rows.
    #[arg(long, default_value_t = 40)]
    rows: u16,
    /// With --exec: a terminal without kitty graphics (image storage disabled), to check
    /// fallbacks.
    #[arg(long)]
    no_graphics: bool,
    /// With --exec: also save everything the command wrote to this file.
    #[arg(long)]
    raw: Option<PathBuf>,
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
    /// Output protocol: kitty, placeholders (kitty via Unicode placeholders), sixel, or the
    /// text fallbacks auto, blocks, half, braille.
    #[arg(long, default_value = "kitty")]
    protocol: String,
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
    if let Some(exec) = &args.exec {
        let argv: Vec<String> = exec.split_whitespace().map(str::to_owned).collect();
        let env: Vec<(String, String)> = args
            .env
            .iter()
            .map(|e| e.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())).expect("--env NAME=VALUE"))
            .collect();
        let keys = kittychart_conformance::pty::parse_keys(&args.keys);
        for th in &themes {
            let (mut t, replies) = new_terminal(th, args.cols, args.rows, cw, ch);
            if args.no_graphics {
                t.set_kitty_image_storage_limit(0).expect("disable kitty graphics");
            }
            let out = kittychart_conformance::pty::run(
                &mut t,
                &replies,
                &argv,
                &env,
                (args.cols, args.rows, cw, ch),
                &keys,
                args.at.map(std::time::Duration::from_millis),
                std::time::Duration::from_secs(60),
            );
            eprintln!("exec {:?}: exit {:?}, {} bytes", th.name, out.exit, out.output.len());
            if let Some(raw) = &args.raw {
                std::fs::write(raw, &out.output).expect("write --raw");
            }
            shots
                .push((format!("{exec} · {}", th.name), paint(&t, th, args.cols, args.rows, cw, ch, &fonts)));
        }
    }
    for file in &args.files {
        let json = std::fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let config = ChartConfig::from_json(&json).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let name = file.file_name().map_or("chart".into(), |n| n.to_string_lossy().into_owned());
        for th in &themes {
            let shot = session(
                &config,
                &name,
                th,
                args.cols,
                cw,
                ch,
                args.chart_cols,
                args.chartjs_colors,
                &args.protocol,
                &fonts,
            );
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
    protocol: &str,
    fonts: &Arc<Mutex<FontContext>>,
) -> Shot {
    // First lay out the chart to learn how many rows the session needs.
    let geom = TermGeometry { cols, cell_width: cw as f32, cell_height: ch as f32 };
    let opts = TermOptions { cols: chart_cols, ..TermOptions::default() };
    let rows = render_for_terminal(config.clone(), geom, &opts).rows + 3;

    let (mut t, replies) = new_terminal(th, cols, rows, cw, ch);

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
    let opts = TermOptions { colors: colors.clone(), ..opts };
    let mut sixel = None;
    let out = if protocol == "kitty" || protocol == "placeholders" {
        render_for_terminal(
            config.clone(),
            geom,
            &TermOptions { placeholders: protocol == "placeholders", ..opts },
        )
    } else if protocol == "sixel" {
        // Ghostty has no sixel: decode our bytes independently and paint them where the
        // cursor was, after moving the cursor past the image as a sixel terminal would.
        let r = render_for_terminal(config.clone(), geom, &TermOptions { protocol: Protocol::Sixel, ..opts });
        let at = (t.cursor_x().unwrap_or(0), t.cursor_y().unwrap_or(0));
        sixel = Some((at, kittychart_testkit::sixel::decode(&r.bytes)));
        kittychart::terminal::TermRender { bytes: "\n".repeat(usize::from(r.rows)).into_bytes(), ..r }
    } else {
        use kittychart_canvas::cells::CellMode;
        let mode = match protocol {
            "braille" => CellMode::Braille,
            "half" => CellMode::HalfBlocks,
            "blocks" => CellMode::Blocks,
            _ => CellMode::Auto,
        };
        let full = render_for_terminal(config.clone(), geom, &opts); // same footprint as kitty
        let grid = kittychart::render_cells(
            config.clone(),
            u32::from(full.cols),
            u32::from(full.rows),
            mode,
            colors.as_ref().map(kittychart::Theme::from_terminal),
        );
        kittychart::terminal::TermRender { bytes: format!("{}\n", grid.to_ansi()).into_bytes(), ..full }
    };
    // The tty's ONLCR turns "\n" into "\r\n".
    let bytes: Vec<u8> =
        out.bytes.iter().flat_map(|&b| if b == b'\n' { vec![b'\r', b'\n'] } else { vec![b] }).collect();
    t.vt_write(&bytes);
    t.vt_write("\x1b[34m~/projects/demo\x1b[0m \x1b[32m❯\x1b[0m ".as_bytes());

    let mut shot = paint(&t, th, cols, rows, cw, ch, fonts);
    if let Some(((x, y), img)) = sixel {
        let pad = cw as usize;
        let (sw, _) = (shot.0 as usize, shot.1 as usize);
        for py in 0..img.height as usize {
            for px in 0..img.width as usize {
                let s = (py * img.width as usize + px) * 4;
                if img.rgba[s + 3] == 0 {
                    continue;
                }
                let (dx, dy) =
                    (pad + usize::from(x) * cw as usize + px, pad + usize::from(y) * ch as usize + py);
                let d = (dy * sw + dx) * 4;
                if d + 4 <= shot.2.len() {
                    shot.2[d..d + 4].copy_from_slice(&img.rgba[s..s + 4]);
                }
            }
        }
    }
    shot
}

/// A simulated terminal with `th`'s colors that answers queries (DA1, cell size, kitty
/// graphics, OSC colors); replies collect in the returned buffer.
fn new_terminal(
    th: &Theme,
    cols: u16,
    rows: u16,
    cw: u32,
    ch: u32,
) -> (Terminal<'static, 'static>, Rc<RefCell<Vec<u8>>>) {
    let colors = kittychart_conformance::Colors {
        background: th.background,
        foreground: th.foreground,
        cursor: th.cursor,
        palette: th.palette,
    };
    kittychart_conformance::answering_terminal(cols, rows, cw, ch, &colors)
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
            if g.first() == Some(&kitty::PLACEHOLDER) {
                // A Unicode placeholder: draw this cell's tile of the image its color names.
                let fg = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                let id = (u32::from(fg.r) << 16) | (u32::from(fg.g) << 8) | u32::from(fg.b);
                let (row, col) = (
                    g.get(1).and_then(|&d| kitty::diacritic_index(d)).unwrap_or(0),
                    g.get(2).and_then(|&d| kitty::diacritic_index(d)).unwrap_or(0),
                );
                let gr = t.kitty_graphics().expect("graphics");
                if let Some(img) = gr.image(id) {
                    let (iw, ih) = (img.width().expect("w"), img.height().expect("h"));
                    let data = img.data().expect("data");
                    let mut tile = vec![0u8; (cw * ch * 4) as usize];
                    for ty in 0..ch {
                        for tx in 0..cw {
                            let (sx, sy) = (col * cw + tx, row * ch + ty);
                            if sx < iw && sy < ih {
                                let s = ((sy * iw + sx) * 4) as usize;
                                let d = ((ty * cw + tx) * 4) as usize;
                                tile[d..d + 4].copy_from_slice(&data[s..s + 4]);
                            }
                        }
                    }
                    c.draw_rgba_device(px, py, cw as f32, ch as f32, &tile, cw, ch);
                }
            } else if !g.is_empty() {
                let style = cell.style().ok();
                let fg = cell.fg_color().ok().flatten().unwrap_or(colors.foreground);
                c.set_font(Font {
                    family: FONT_STACK.into(),
                    size: font_px,
                    weight: if style.is_some_and(|s| s.bold) { 700 } else { 400 },
                    italic: false,
                });
                c.set_fill_style(Paint::Solid(col(fg)));
                let first = g[0];
                if !draw_block_glyph(&mut c, first, px, py, cw as f32, ch as f32) {
                    let s: String = g.into_iter().collect();
                    c.fill_text(&s, px, py + ch as f32 / 2.0, None);
                }
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

/// Draw block elements and Braille as geometry, as terminals do (Ghostty and kitty render
/// them as sprites that tile seamlessly, not with the font). Uses the current fill style.
fn draw_block_glyph(c: &mut SkiaCanvas, ch: char, x: f32, y: f32, w: f32, h: f32) -> bool {
    let cp = ch as u32;
    let rect = |c: &mut SkiaCanvas, fx: f32, fy: f32, fw: f32, fh: f32| {
        c.fill_rect(x + fx * w, y + fy * h, fw * w, fh * h)
    };
    match cp {
        0x2588 => rect(c, 0.0, 0.0, 1.0, 1.0), // █
        0x2580 => rect(c, 0.0, 0.0, 1.0, 0.5), // ▀
        0x2584 => rect(c, 0.0, 0.5, 1.0, 0.5), // ▄
        0x258C => rect(c, 0.0, 0.0, 0.5, 1.0), // ▌
        0x2590 => rect(c, 0.5, 0.0, 0.5, 1.0), // ▐
        0x2596..=0x259F => {
            // Quadrants: bit 0 TL, 1 TR, 2 BL, 3 BR.
            let mask: u32 = match cp {
                0x2596 => 0b0100, // ▖
                0x2597 => 0b1000, // ▗
                0x2598 => 0b0001, // ▘
                0x2599 => 0b1101, // ▙
                0x259A => 0b1001, // ▚
                0x259B => 0b0111, // ▛
                0x259C => 0b1011, // ▜
                0x259D => 0b0010, // ▝
                0x259E => 0b0110, // ▞
                _ => 0b1110,      // ▟
            };
            for (bit, (fx, fy)) in [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)].iter().enumerate() {
                if mask & (1 << bit) != 0 {
                    rect(c, *fx, *fy, 0.5, 0.5);
                }
            }
        }
        0x2801..=0x28FF => {
            let bits = cp - 0x2800;
            const POS: [(u32, f32, f32); 8] = [
                (0x01, 0.0, 0.0),
                (0x02, 0.0, 1.0),
                (0x04, 0.0, 2.0),
                (0x40, 0.0, 3.0),
                (0x08, 1.0, 0.0),
                (0x10, 1.0, 1.0),
                (0x20, 1.0, 2.0),
                (0x80, 1.0, 3.0),
            ];
            let r = (w / 2.0).min(h / 4.0) * 0.32;
            for (bit, dx, dy) in POS {
                if bits & bit != 0 {
                    c.begin_path();
                    c.arc(
                        x + (dx + 0.5) * w / 2.0,
                        y + (dy + 0.5) * h / 4.0,
                        r,
                        0.0,
                        std::f32::consts::TAU,
                        false,
                    );
                    c.fill();
                }
            }
        }
        _ => return false,
    }
    true
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
