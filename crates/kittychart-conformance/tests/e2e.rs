//! End to end: the real `kittychart` binary, the ratatui example and tmux, each on a real
//! PTY whose other end is Ghostty's VT core. These cover what unit tests can't: the TTY adapter
//! (raw mode, timeouts), detection against a terminal that actually answers, protocol choice,
//! cursor placement, and tmux passthrough.

mod common;

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use common::*;

#[test]
fn cli_draws_kitty_graphics_and_leaves_the_cursor_below() {
    let r = run(&[cli(), fixture("bar/grouped.json"), "--cols".into(), "60".into()], true, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    let (cols, rows) = p[0].grid;
    assert!(!p[0].virtual_);
    assert_eq!(p[0].at, Some((0, 0)));
    assert_eq!(cols, 60);
    assert_eq!(
        (r.term.cursor_x().unwrap(), u32::from(r.term.cursor_y().unwrap())),
        (0, rows),
        "cursor on the line after the image"
    );
}

#[test]
fn cli_falls_back_to_text_without_graphics() {
    let r = run(&[cli(), fixture("line/basic.json"), "--cols".into(), "80".into()], false, &[], None);
    assert_eq!(r.out.exit, Some(0));
    assert!(placements(&r.term).is_empty());
    let text = screen_text(&r.term);
    for label in ["Line Chart", "My First Dataset", "January", "July"] {
        assert!(text.contains(label), "{label:?} missing from:\n{text}");
    }
    let braille = text.chars().filter(|c| ('\u{2801}'..='\u{28FF}').contains(c)).count();
    assert!(braille > 100, "lines are Braille ({braille} cells):\n{text}");
}

#[test]
fn cli_in_tmux_uses_passthrough_and_placeholders() {
    let tmux = Command::new("tmux").arg("-V").output();
    assert!(tmux.is_ok_and(|o| o.status.success()), "tmux not found: install tmux to run this test");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("e2e-tmux");
    std::fs::create_dir_all(&dir).unwrap();
    let conf = dir.join("tmux.conf");
    std::fs::write(
        &conf,
        "set -g allow-passthrough on\nset -g default-terminal tmux-256color\n\
         set -as terminal-features ',*:RGB'\nset -g status off\n",
    )
    .unwrap();
    let socket = format!("kc-e2e-{}", std::process::id());
    let cmd = format!("{} {} --cols 60; sleep 5", cli(), fixture("bar/grouped.json"));
    let argv: Vec<String> = ["tmux", "-L", &socket, "-f", conf.to_str().unwrap(), "new-session", &cmd]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = run(&argv, true, &[], Some(3000));
    let _ = Command::new("tmux").args(["-L", &socket, "kill-server"]).status();

    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "one image, through passthrough: {p:?}");
    assert!(p[0].virtual_, "a virtual placement, shown by placeholder cells");
    let (cols, rows) = p[0].grid;
    assert_eq!(cols, 60);
    assert_eq!(placeholder_cells(&r.term), (cols * rows) as usize, "tmux drew every placeholder cell");
}

#[test]
fn ratatui_dashboard_draws_both_charts_and_cleans_up() {
    let bin = built(&["-p", "kittychart-ratatui", "--example", "dashboard"], "examples/dashboard");
    let r = run(std::slice::from_ref(&bin), true, &[], Some(3000));
    let mut p = placements(&r.term);
    p.sort_by_key(|p| p.image_id);
    assert_eq!(p.iter().map(|p| (p.image_id, p.virtual_)).collect::<Vec<_>>(), [(1, true), (2, true)]);
    let expected: u32 = p.iter().map(|p| p.grid.0 * p.grid.1).sum();
    assert_eq!(placeholder_cells(&r.term), expected as usize, "every chart cell is a placeholder");

    // q quits: back to the main screen, images freed.
    let keys = [Key { at: Duration::from_millis(2500), bytes: b"q".to_vec() }];
    let r = run(&[bin], true, &keys, None);
    assert_eq!(r.out.exit, Some(0));
    let g = r.term.kitty_graphics().unwrap();
    assert!(g.image(1).is_none() && g.image(2).is_none(), "images deleted on exit");
}

#[test]
fn c_api_show_draws_kitty_graphics() {
    let r = run(&[c_example(), fixture("line/basic.json")], true, &[], None);
    assert_eq!(r.out.exit, Some(0), "{}", String::from_utf8_lossy(&r.out.output));
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(!p[0].virtual_);
    assert_eq!(p[0].grid.0, 60, "kc_show_options.cols");
    assert_eq!(u32::from(r.term.cursor_y().unwrap()), p[0].grid.1, "cursor below the image");
}

#[test]
fn c_api_show_falls_back_to_text() {
    let r = run(&[c_example()], false, &[], None);
    assert_eq!(r.out.exit, Some(0));
    assert!(placements(&r.term).is_empty());
    assert!(screen_text(&r.term).contains("Requests"));
}

/// Wider than the 297 placeholder diacritics: columns past them are inferred by the terminal
/// (found on a real 540-column Ghostty window, where the chart repeated itself).
#[test]
fn tmux_placeholders_wider_than_the_diacritic_table() {
    let tmux = Command::new("tmux").arg("-V").output();
    assert!(tmux.is_ok_and(|o| o.status.success()), "tmux not found: install tmux to run this test");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("e2e-tmux-wide");
    std::fs::create_dir_all(&dir).unwrap();
    let conf = dir.join("tmux.conf");
    std::fs::write(
        &conf,
        "set -g allow-passthrough on\nset -g default-terminal tmux-256color\n\
         set -as terminal-features ',*:RGB'\nset -g status off\n",
    )
    .unwrap();
    let socket = format!("kc-e2e-wide-{}", std::process::id());
    let cmd = format!("{} {} --rows 20; sleep 5", cli(), fixture("line/basic.json"));
    let argv: Vec<String> = ["tmux", "-L", &socket, "-f", conf.to_str().unwrap(), "new-session", &cmd]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let r = run_geometry(&argv, 400, 32, true, &[], Some(3000));
    let _ = Command::new("tmux").args(["-L", &socket, "kill-server"]).status();
    let p = placements(&r.term);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(p[0].grid, (400, 20));
    assert_whole_images(&r.term, &p);
}
