//! End to end: `kittychart view` (the interactive viewer) on a PTY whose other end is
//! Ghostty's VT core. Mouse input is typed as real SGR pixel reports (mode 1016).

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::*;
use kittychart::canvas::SkiaCanvas;
use kittychart::{Chart, ChartConfig};

const CONFIG: &str = r##"{
  "type": "bar",
  "data": {
    "labels": ["Q1", "Q2", "Q3", "Q4"],
    "datasets": [
      { "label": "Revenue", "data": [12, 19, 7, 15], "backgroundColor": "#ff0000" },
      { "label": "Costs", "data": [8, 11, 14, 9], "backgroundColor": "#0000ff" }
    ]
  },
  "options": { "interaction": { "mode": "index", "intersect": false }, "animation": false }
}"##;

fn write_config(name: &str, json: &str) -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("view");
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(format!("{name}-{}.json", std::process::id()));
    std::fs::write(&p, json).unwrap();
    p.to_string_lossy().into_owned()
}

/// The legend item boxes of `json` laid out as the viewer lays it out (CSS px).
fn legend_boxes(json: &str) -> (Vec<[f64; 4]>, f64) {
    let scale = kittychart::terminal::default_scale(CH as f32);
    let (w, h) = (u32::from(COLS) * CW, u32::from(ROWS - 1) * CH);
    let mut chart = Chart::new(ChartConfig::from_json(json).unwrap());
    chart.dpr = f64::from(scale);
    chart.draw(&mut SkiaCanvas::with_pixel_size(w, h, scale));
    (chart.legend.item_boxes(), f64::from(scale))
}

fn mouse_move(x: u32, y: u32) -> Vec<u8> {
    format!("\x1b[<35;{};{}M", x + 1, y + 1).into_bytes()
}

fn click(x: u32, y: u32) -> Vec<u8> {
    format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1).into_bytes()
}

fn key(ms: u64, bytes: Vec<u8>) -> Key {
    Key { at: Duration::from_millis(ms), bytes }
}

fn the_image(r: &Run) -> (u32, Vec<u8>) {
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "one image, one placement: {p:?}");
    let (_, _, rgba) = image_rgba(&r.term, p[0].image_id).unwrap();
    (p[0].image_id, rgba)
}

/// Tooltip background: rgba(0,0,0,0.8).
fn tooltip_pixels(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 150 && p[3] < 240 && p[0] < 10 && p[1] < 10 && p[2] < 10)
        .count()
}

#[test]
fn hover_shows_a_tooltip_and_q_cleans_up() {
    let file = write_config("hover", CONFIG);
    let argv = vec![cli(), "view".into(), file];
    let still = run(&argv, true, &[], Some(2000));
    let (_, plain) = the_image(&still);
    assert_eq!(tooltip_pixels(&plain), 0);

    let hovered = run(&argv, true, &[key(1200, mouse_move(500, 300))], Some(2500));
    let (_, rgba) = the_image(&hovered);
    assert!(tooltip_pixels(&rgba) > 1000, "a tooltip box: {} px", tooltip_pixels(&rgba));
    let text = screen_text(&hovered.term);
    assert!(text.contains("q quit"), "status line:\n{text}");

    let quit = run(&argv, true, &[key(1200, mouse_move(500, 300)), key(2000, b"q".to_vec())], None);
    assert_eq!(quit.out.exit, Some(0), "{}", String::from_utf8_lossy(&quit.out.output));
    assert!(placements(&quit.term).is_empty(), "image deleted on exit");
    assert!(!screen_text(&quit.term).contains("q quit"), "back on the main screen");
}

#[test]
fn clicking_the_legend_hides_a_dataset() {
    let file = write_config("legend", CONFIG);
    let (boxes, scale) = legend_boxes(CONFIG);
    let b = boxes[1]; // "Costs"
    let (x, y) = (((b[0] + b[2] / 2.0) * scale) as u32, ((b[1] + b[3] / 2.0) * scale) as u32);
    let argv = vec![cli(), "view".into(), file];
    let before = run(&argv, true, &[], Some(2000));
    let (_, rgba) = the_image(&before);
    let blue_before = count_color(&rgba, [0, 0, 255], 8);
    let after = run(&argv, true, &[key(1200, click(x, y))], Some(2500));
    let (_, rgba) = the_image(&after);
    let blue_after = count_color(&rgba, [0, 0, 255], 8);
    assert!(
        blue_before > 10_000 && blue_after < blue_before / 20,
        "blue bars hidden: {blue_before} → {blue_after}"
    );
    assert!(count_color(&rgba, [255, 0, 0], 8) > 10_000, "red bars stay");
}

#[test]
fn text_mode_tooltip() {
    let file = write_config("text", CONFIG);
    let argv = vec![cli(), "view".into(), file, "--protocol".into(), "text".into()];
    // Cell coordinates: no graphics, so pixel mouse is still reported but mapped to cells.
    let r = run(&argv, false, &[key(1200, mouse_move(500, 300))], Some(2500));
    let text = screen_text(&r.term);
    assert!(text.contains("Revenue:") && text.contains("Costs:"), "tooltip lines:\n{text}");
}

#[test]
fn watch_animates_to_the_new_file() {
    let file = write_config("watch", CONFIG);
    let changed = CONFIG
        .replace("[12, 19, 7, 15]", "[1, 2, 3, 4]")
        .replace(r#""animation": false"#, r#""animation": {"duration": 300}"#);
    let f2 = file.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(1500));
        std::fs::write(&f2, changed).unwrap();
    });
    let argv = vec![cli(), "view".into(), file, "--watch".into()];
    let before = run(&argv, true, &[], Some(1200));
    let (_, rgba) = the_image(&before);
    let red_before = count_color(&rgba, [255, 0, 0], 8);
    let r = run(&argv, true, &[], Some(3500));
    writer.join().unwrap();
    let (_, rgba) = the_image(&r);
    let red_after = count_color(&rgba, [255, 0, 0], 8);
    assert!(red_after < red_before / 2, "red bars shrank to the new data: {red_before} → {red_after}");
}
