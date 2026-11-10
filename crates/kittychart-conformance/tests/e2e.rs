//! End to end: the real `kittychart` binary, the ratatui example and tmux, each on a real
//! PTY whose other end is Ghostty's VT core. These cover what unit tests can't: the TTY adapter
//! (raw mode, timeouts), detection against a terminal that actually answers, protocol choice,
//! cursor placement, and tmux passthrough.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

use kittychart_conformance::pty::{self, Key, Outcome};
use kittychart_conformance::{Colors, answering_terminal, screen_text};
use libghostty_vt::Terminal;
use libghostty_vt::kitty::graphics::PlacementIterator;

const COLS: u16 = 100;
const ROWS: u16 = 32;
const CW: u32 = 10;
const CH: u32 = 20;

fn root() -> PathBuf {
    kittychart_testkit::workspace_root()
}

/// Build a workspace binary (once per process; cargo serializes concurrent builds itself) and
/// return its path.
fn built(cargo_args: &[&str], rel: &str) -> String {
    static BUILD: Mutex<()> = Mutex::new(());
    let _guard = BUILD.lock().unwrap();
    let status = Command::new(env!("CARGO"))
        .args(["build", "--quiet"])
        .args(cargo_args)
        .current_dir(root())
        .status()
        .expect("cargo build");
    assert!(status.success(), "building {cargo_args:?} failed");
    // CARGO_TARGET_TMPDIR is <target>/tmp.
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).parent().unwrap().to_path_buf();
    target.join("debug").join(rel).to_string_lossy().into_owned()
}

fn cli() -> String {
    built(&["-p", "kittychart-cli"], "kittychart")
}

fn fixture(name: &str) -> String {
    root().join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

struct Run {
    term: Terminal<'static, 'static>,
    out: Outcome,
}

fn run(argv: &[String], graphics: bool, keys: &[Key], snapshot_ms: Option<u64>) -> Run {
    let (mut term, replies) = answering_terminal(COLS, ROWS, CW, CH, &Colors::default());
    if !graphics {
        term.set_kitty_image_storage_limit(0).unwrap();
    }
    let out = pty::run(
        &mut term,
        &replies,
        argv,
        &[],
        (COLS, ROWS, CW, CH),
        keys,
        snapshot_ms.map(Duration::from_millis),
        Duration::from_secs(60),
    );
    Run { term, out }
}

#[derive(Debug)]
struct Placed {
    image_id: u32,
    virtual_: bool,
    at: Option<(i32, i32)>,
    grid: (u32, u32),
}

fn placements(t: &Terminal<'_, '_>) -> Vec<Placed> {
    let g = t.kitty_graphics().unwrap();
    let mut iter = PlacementIterator::new().unwrap();
    let mut it = iter.update(&g).unwrap();
    let mut out = Vec::new();
    while let Some(p) = it.next() {
        let img = g.image(p.image_id().unwrap()).unwrap();
        let grid = p.grid_size(&img, t).unwrap();
        out.push(Placed {
            image_id: p.image_id().unwrap(),
            virtual_: p.is_virtual().unwrap(),
            at: p.viewport_pos(&img, t).ok().flatten().map(|v| (v.col, v.row)),
            grid: (grid.cols, grid.rows),
        });
    }
    out
}

fn placeholder_cells(t: &Terminal<'_, '_>) -> usize {
    screen_text(t).chars().filter(|&c| c == kittychart_term::kitty::PLACEHOLDER).count()
}

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
