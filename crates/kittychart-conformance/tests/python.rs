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
