//! End to end: the Python package (`bindings/python`) in a real interpreter on a PTY whose
//! other end is Ghostty's VT core. Covers detection through the binding, `Chart.show`, Rich
//! layouts and the Textual widget. Needs the virtualenv: `just python-dev`.

mod common;

use std::time::Duration;

use common::*;

fn py(args: &[&str]) -> Vec<String> {
    std::iter::once(python()).chain(args.iter().map(|s| s.to_string())).collect()
}

#[test]
fn show_draws_kitty_graphics_with_numpy_data() {
    let r = run(&py(&[&python_example("show.py")]), true, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(!p[0].virtual_);
    assert_eq!(p[0].grid.0, 70, "cols=70");
    assert_eq!(u32::from(r.term.cursor_y().unwrap()), p[0].grid.1, "cursor below the image");
}

#[test]
fn show_falls_back_to_text() {
    let r = run(&py(&[&python_example("show.py")]), false, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    assert!(placements(&r.term).is_empty());
    let text = screen_text(&r.term);
    assert!(text.contains("numpy → Chart.js → terminal"), "{text}");
}

#[test]
fn rich_layouts_hold_images_as_placeholders() {
    let r = run_sized(&py(&[&python_example("rich_report.py")]), 50, true, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 2, "panel + table: {p:?}");
    assert!(p.iter().all(|p| p.virtual_));
    assert_whole_images(&r.term, &p);
    let text = screen_text(&r.term);
    assert!(text.contains("Quarterly revenue") && text.contains("Web still leads"), "{text}");
}

#[test]
fn rich_falls_back_to_text() {
    let r = run_sized(&py(&[&python_example("rich_report.py")]), 50, false, &[], None);
    assert_eq!(r.out.exit, Some(0));
    assert!(placements(&r.term).is_empty());
    let text = screen_text(&r.term);
    for s in ["Quarterly revenue", "Revenue", "Q4", "Android"] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
}

#[test]
fn textual_dashboard_draws_charts_updates_and_cleans_up() {
    let argv = py(&[&python_example("textual_dashboard.py")]);
    let r = run(&argv, true, &[], Some(4000));
    let p = placements(&r.term);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(p.iter().all(|p| p.virtual_));
    // Each chart fills its half of the screen, and every image cell is visible.
    for p in &p {
        assert_eq!(p.grid.0, u32::from(COLS) / 2, "{p:?}");
    }
    assert_whole_images(&r.term, &p);

    // Space: new data → new images; q: quit, images deleted.
    let keys = [
        Key { at: Duration::from_millis(3000), bytes: b" ".to_vec() },
        Key { at: Duration::from_millis(4500), bytes: b"q".to_vec() },
    ];
    let r = run(&argv, true, &keys, None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    assert!(placements(&r.term).is_empty(), "images deleted on exit");
}

/// New data animates through many frames, each under the other of the widget's two image ids
/// (double buffering). Afterwards every placeholder must still name an image that exists:
/// freeing the replaced image before Textual has written the lines that switch away from it
/// blanks the chart.
#[test]
fn textual_updates_keep_every_chart_visible() {
    let argv = py(&[&python_example("textual_dashboard.py")]);
    let env = [("KITTYCHART_EXAMPLE_SEED", "7")];
    let keys: Vec<Key> = [3000, 3600, 4200, 4800, 5400, 6000]
        .map(|ms| Key { at: Duration::from_millis(ms), bytes: b" ".to_vec() })
        .to_vec();
    let r = run_full(&argv, &env, COLS, ROWS, true, &keys, Some(8500));
    let p = placements(&r.term);
    assert_eq!(p.len(), 2, "one image per chart: {p:?}");
    assert_placeholders_name_live_images(&r.term);
    assert_whole_images(&r.term, &p);
}

/// The opening animation runs by itself (its timer starts on mount, with no input): about a
/// second at 30 frames per second, so the charts go out many times, and the bars end at
/// their full height (seed 7: the tallest Signups bar is 39 on a 0–40 axis, in the top
/// quarter).
#[test]
fn textual_initial_animation_runs() {
    let argv = py(&[&python_example("textual_dashboard.py")]);
    let env = [("KITTYCHART_EXAMPLE_SEED", "7")];
    let r = run_full(&argv, &env, COLS, ROWS, true, &[], Some(6000));
    let bar = placements(&r.term).into_iter().map(|p| p.image_id).min().expect("bar chart");
    // Both charts' frames (each alternates between its id and that id's partner).
    let frames = kittychart_testkit::kitty::decode_transmissions(&r.out.output).unwrap().len();
    assert!(frames >= 20, "{frames} frames went out: no animation");
    let (w, h, rgba) = image_rgba(&r.term, bar).unwrap();
    let blue_on_top = rgba[..(w * h / 4 * 4) as usize]
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 200 && p[2] > 150 && p[0] < 100)
        .count();
    assert!(blue_on_top > 100, "the bars stopped short of their final height ({blue_on_top} px up top)");
}

#[test]
fn textual_falls_back_to_text() {
    let r = run(&py(&[&python_example("textual_dashboard.py")]), false, &[], Some(4000));
    assert!(placements(&r.term).is_empty());
    let text = screen_text(&r.term);
    for s in ["Signups", "Latency (ms)", "Mon", "Sun"] {
        assert!(text.contains(s), "{s:?} missing:\n{text}");
    }
}

fn ipython() -> Vec<String> {
    let ipy = root().join("bindings/python/.venv/bin/ipython").to_string_lossy().into_owned();
    [ipy.as_str(), "--no-banner", "--colors=nocolor", "-i", &python_example("ipython_session.py")]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn ipython_displays_charts_below_the_out_prompt() {
    let keys = [
        Key { at: Duration::from_millis(3000), bytes: b"c\r".to_vec() },
        Key { at: Duration::from_millis(5000), bytes: b"exit\r".to_vec() },
    ];
    let r = run(&ipython(), true, &keys, None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    let text = screen_text(&r.term);
    let out_row = text.lines().position(|l| l.starts_with("Out[1]:")).expect(&text) as i32;
    let (col, row) = p[0].at.expect("on screen");
    assert_eq!(col, 0, "the image starts at the left margin, not after the prompt");
    assert_eq!(row, out_row + 1, "the image starts on the line after Out[1]:\n{text}");
    assert_eq!(p[0].grid.0, u32::from(COLS));
    assert!(text.contains("In [2]: exit"), "{text}");
}

#[test]
fn ipython_falls_back_to_text() {
    let keys = [
        Key { at: Duration::from_millis(3000), bytes: b"c\r".to_vec() },
        Key { at: Duration::from_millis(5000), bytes: b"exit\r".to_vec() },
    ];
    let r = run(&ipython(), false, &keys, None);
    assert_eq!(r.out.exit, Some(0));
    let text = screen_text(&r.term);
    assert!(text.contains("Revenue") && text.contains("Q3"), "{text}");
}

/// Sixel in Rich layouts: the chart's cells are reserved as blank text and the image drawn over
/// them afterwards, so borders and neighbouring cells stay put. Ghostty ignores the sixel data
/// itself, which leaves exactly the layout to check (the image was checked in a real foot).
#[test]
fn rich_sixel_keeps_the_layout() {
    let r = run_env(&py(&[&python_example("rich_report.py")]), &[("KITTYCHART_PROTOCOL", "sixel")], 50);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    assert!(String::from_utf8_lossy(&r.out.output).contains("\x1bP0;1;0q"), "sixel was sent");
    let text = screen_text(&r.term);
    let lines: Vec<&str> = text.lines().collect();
    let top = lines.iter().position(|l| l.contains("Quarterly revenue")).expect(&text);
    let bottom = lines.iter().skip(top + 1).position(|l| l.starts_with('╰')).expect(&text) + top + 1;
    assert!(bottom - top > 5, "the panel holds the chart's rows:\n{text}");
    for l in &lines[top + 1..bottom] {
        let l = l.trim_end();
        assert!(l.starts_with('│') && l.ends_with('│') && l.chars().count() == 60, "panel row {l:?}\n{text}");
    }
    let notes = lines.iter().find(|l| l.contains("Web still leads")).expect(&text);
    assert!(notes.find("Web still leads").unwrap() > 40, "notes stay in their column:\n{text}");
}

fn tooltip_px(rgba: &[u8]) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 150 && p[3] < 240 && p[0] < 10 && p[1] < 10 && p[2] < 10)
        .count()
}

#[test]
fn textual_hover_adds_a_tooltip() {
    let argv = py(&[&python_example("textual_dashboard.py")]);
    // The example's data is random unless seeded: the same seed gives the still and the
    // hovered run the same charts, so they can be compared pixel for pixel.
    let env = [("KITTYCHART_EXAMPLE_SEED", "7")];
    let images = |r: &Run| -> Vec<(u32, Vec<u8>)> {
        let mut v: Vec<_> = placements(&r.term)
            .iter()
            .map(|p| (p.image_id, image_rgba(&r.term, p.image_id).unwrap().2))
            .collect();
        // Ids follow mount order (the bar chart, then the line chart); each chart may be on its
        // double-buffering partner id, which only differs in bit 24.
        v.sort_by_key(|(id, _)| id & 0x00FF_FFFF);
        v
    };
    let still = images(&run_full(&argv, &env, COLS, ROWS, true, &[], Some(6000)));
    assert_eq!(still.len(), 2);
    // Move into the right-hand (line) chart, in cell coordinates, after the initial animation
    // has finished (so the hover transition must drive its own frames). Python + Textual can
    // take a few seconds to start under a loaded test run, so the move repeats until it lands.
    let mv: Vec<Key> = [4500, 6000]
        .map(|ms| Key { at: Duration::from_millis(ms), bytes: b"\x1b[<35;75;16M".to_vec() })
        .to_vec();
    let hovered = images(&run_full(&argv, &env, COLS, ROWS, true, &mv, Some(7500)));
    assert_eq!(hovered.len(), 2);
    let changed: Vec<usize> = still.iter().zip(&hovered).map(|((_, a), (_, b))| darkened_px(a, b)).collect();
    assert!(changed[0] < 500, "the bar chart changed: {changed:?}");
    assert!(changed[1] > 2000, "no tooltip on the line chart: {changed:?}");
}

/// Pixels that `b` draws clearly darker than `a`, both composited over mid-grey: a tooltip box
/// (translucent black) shows up whether it covers the background, a fill or a line.
fn darkened_px(a: &[u8], b: &[u8]) -> usize {
    let luma = |p: &[u8; 4]| -> i32 {
        let alpha = i32::from(p[3]);
        (0..3).map(|i| (i32::from(p[i]) * alpha + 128 * (255 - alpha)) / 255).sum()
    };
    a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0).filter(|(p, q)| luma(p) - luma(q) > 90).count()
}

#[test]
fn python_interact_hover_and_quit() {
    let argv = py(&[&python_example("interact.py")]);
    let keys = [Key { at: Duration::from_millis(1500), bytes: b"\x1b[<35;500;300M".to_vec() }];
    let hovered = run(&argv, true, &keys, Some(2500));
    let p = placements(&hovered.term);
    assert_eq!(p.len(), 1, "{p:?}");
    let (_, _, rgba) = image_rgba(&hovered.term, p[0].image_id).unwrap();
    assert!(tooltip_px(&rgba) > 1000, "tooltip: {} px", tooltip_px(&rgba));
    let quit = run(
        &argv,
        true,
        &[keys[0].clone(), Key { at: Duration::from_millis(2200), bytes: b"q".to_vec() }],
        None,
    );
    assert_eq!(quit.out.exit, Some(0), "{}", String::from_utf8_lossy(&quit.out.output));
    assert!(placements(&quit.term).is_empty());
}
