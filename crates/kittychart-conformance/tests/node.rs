//! End to end: the npm package (`bindings/node`) in real Node.js on a PTY whose other end is
//! Ghostty's VT core. Needs the native module built: `just node-dev`.

mod common;

use common::*;

fn node_show() -> Vec<String> {
    let dir = root().join("bindings/node");
    assert!(
        std::fs::read_dir(&dir).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "node")),
        "no native module in {}: run `just node-dev` first",
        dir.display()
    );
    vec!["node".into(), dir.join("examples/show.js").to_string_lossy().into_owned()]
}

#[test]
fn show_draws_kitty_graphics_with_typed_array_data() {
    let r = run(&node_show(), true, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(!p[0].virtual_);
    assert_eq!(p[0].at, Some((0, 0)));
    assert_eq!(p[0].grid.0, 70);
    assert_eq!(u32::from(r.term.cursor_y().unwrap()), p[0].grid.1, "cursor below the image");
}

#[test]
fn show_falls_back_to_text() {
    let r = run(&node_show(), false, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    assert!(placements(&r.term).is_empty());
    let text = screen_text(&r.term);
    assert!(text.contains("Node → Chart.js → terminal") && text.contains("CPU %"), "{text}");
}

#[test]
fn interact_hover_with_js_callbacks_and_quit() {
    let script = root().join("bindings/node/examples/interact.js").to_string_lossy().into_owned();
    let argv = vec!["node".to_owned(), script];
    let tooltip = |rgba: &[u8]| {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] > 150 && p[3] < 240 && p[0] < 10 && p[1] < 10 && p[2] < 10)
            .count()
    };
    let mv = Key { at: std::time::Duration::from_millis(1500), bytes: b"\x1b[<35;500;300M".to_vec() };
    let r = run(&argv, true, std::slice::from_ref(&mv), Some(2500));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}\n{}", String::from_utf8_lossy(&r.out.output));
    let (_, _, rgba) = image_rgba(&r.term, p[0].image_id).unwrap();
    assert!(tooltip(&rgba) > 1000, "tooltip: {} px", tooltip(&rgba));
    // The JS function colored the Revenue bars (> 10 → #ff0000, else #ff8080).
    assert!(count_color(&rgba, [255, 0, 0], 6) > 2000 && count_color(&rgba, [255, 128, 128], 6) > 500);
    let quit = run(
        &argv,
        true,
        &[mv, Key { at: std::time::Duration::from_millis(2200), bytes: b"q".to_vec() }],
        None,
    );
    assert_eq!(quit.out.exit, Some(0), "{}", String::from_utf8_lossy(&quit.out.output));
    assert!(placements(&quit.term).is_empty());
}
