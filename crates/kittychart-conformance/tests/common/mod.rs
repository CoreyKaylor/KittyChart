//! Helpers shared by the end-to-end tests: build workspace binaries, run programs on a PTY
//! attached to Ghostty's VT core, and inspect the resulting screen and kitty images.
#![allow(dead_code)] // each test binary uses a different subset

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

pub use kittychart_conformance::pty::{self, Key, Outcome};
pub use kittychart_conformance::{Colors, answering_terminal, screen_text};
use libghostty_vt::Terminal;
use libghostty_vt::kitty::graphics::PlacementIterator;

pub const COLS: u16 = 100;
pub const ROWS: u16 = 32;
pub const CW: u32 = 10;
pub const CH: u32 = 20;

pub fn root() -> PathBuf {
    kittychart_testkit::workspace_root()
}

/// Build a workspace binary (once per process; cargo serializes concurrent builds itself) and
/// return its path.
pub fn built(cargo_args: &[&str], rel: &str) -> String {
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

pub fn cli() -> String {
    built(&["-p", "kittychart-cli"], "kittychart")
}

/// The C example (`crates/kittychart-ffi/examples/show.c`), compiled against the static library
/// once per process. nextest runs each test in its own process, so the binary name includes the
/// pid: a shared path would be rewritten while another test runs it.
pub fn c_example() -> String {
    static EXE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    EXE.get_or_init(compile_c_example).clone()
}

pub fn compile_c_example() -> String {
    let lib = built(&["-p", "kittychart-ffi"], "libkittychart_ffi.a");
    let ffi = root().join("crates/kittychart-ffi");
    let exe = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("show-c-{}", std::process::id()));
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let o = Command::new(&cc)
        .args(["-std=c99", "-Wall", "-Werror", "-o"])
        .arg(&exe)
        .arg(ffi.join("examples/show.c"))
        .arg("-I")
        .arg(ffi.join("include"))
        .arg(lib)
        .args(["-lm", "-lpthread", "-ldl"])
        .output()
        .expect("cc");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    exe.to_string_lossy().into_owned()
}

pub fn fixture(name: &str) -> String {
    root().join("tests/fixtures").join(name).to_string_lossy().into_owned()
}

pub struct Run {
    pub term: Terminal<'static, 'static>,
    pub out: Outcome,
}

pub fn run(argv: &[String], graphics: bool, keys: &[Key], snapshot_ms: Option<u64>) -> Run {
    run_sized(argv, ROWS, graphics, keys, snapshot_ms)
}

/// [`run`] in a terminal `rows` tall.
pub fn run_sized(argv: &[String], rows: u16, graphics: bool, keys: &[Key], snapshot_ms: Option<u64>) -> Run {
    run_geometry(argv, COLS, rows, graphics, keys, snapshot_ms)
}

/// [`run`] with extra environment variables, in a terminal `rows` tall.
pub fn run_env(argv: &[String], env: &[(&str, &str)], rows: u16) -> Run {
    let env: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let (mut term, replies) = answering_terminal(COLS, rows, CW, CH, &Colors::default());
    let out =
        pty::run(&mut term, &replies, argv, &env, (COLS, rows, CW, CH), &[], None, Duration::from_secs(60));
    Run { term, out }
}

/// [`run`] in a `cols × rows` terminal.
pub fn run_geometry(
    argv: &[String],
    cols: u16,
    rows: u16,
    graphics: bool,
    keys: &[Key],
    snapshot_ms: Option<u64>,
) -> Run {
    run_full(argv, &[], cols, rows, graphics, keys, snapshot_ms)
}

/// [`run_geometry`] with extra environment variables.
pub fn run_full(
    argv: &[String],
    env: &[(&str, &str)],
    cols: u16,
    rows: u16,
    graphics: bool,
    keys: &[Key],
    snapshot_ms: Option<u64>,
) -> Run {
    let env: Vec<(String, String)> = env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let (mut term, replies) = answering_terminal(cols, rows, CW, CH, &Colors::default());
    if !graphics {
        term.set_kitty_image_storage_limit(0).unwrap();
    }
    let out = pty::run(
        &mut term,
        &replies,
        argv,
        &env,
        (cols, rows, CW, CH),
        keys,
        snapshot_ms.map(Duration::from_millis),
        Duration::from_secs(60),
    );
    Run { term, out }
}

#[derive(Debug)]
pub struct Placed {
    pub image_id: u32,
    pub virtual_: bool,
    pub at: Option<(i32, i32)>,
    pub grid: (u32, u32),
}

pub fn placements(t: &Terminal<'_, '_>) -> Vec<Placed> {
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

pub fn placeholder_cells(t: &Terminal<'_, '_>) -> usize {
    screen_text(t).chars().filter(|&c| c == kittychart_term::kitty::PLACEHOLDER).count()
}

/// Placeholder cells in each screen row (for checking that an image's cells are all visible).
pub fn placeholder_cells_per_row(t: &Terminal<'_, '_>) -> Vec<usize> {
    screen_text(t)
        .lines()
        .map(|l| l.chars().filter(|&c| c == kittychart_term::kitty::PLACEHOLDER).count())
        .collect()
}

/// The Python binding's virtualenv interpreter (`just python-dev` creates it).
pub fn python() -> String {
    let py = root().join("bindings/python/.venv/bin/python");
    assert!(py.exists(), "{} missing: run `just python-dev` first", py.display());
    py.to_string_lossy().into_owned()
}

pub fn python_example(name: &str) -> String {
    root().join("bindings/python/examples").join(name).to_string_lossy().into_owned()
}

/// For every placeholder cell on screen: `(screen row, screen col, image row, image col)`, with
/// the image position decoded from the cell's diacritics. A missing column diacritic means
/// "the previous cell's column + 1", as the kitty spec says.
pub fn placeholder_positions(t: &Terminal<'_, '_>) -> Vec<(usize, usize, u32, u32)> {
    use kittychart_term::kitty::{PLACEHOLDER, diacritic_index};
    use libghostty_vt::render::{CellIterator, RenderState, RowIterator};
    let mut rs = RenderState::new().unwrap();
    let snap = rs.update(t).unwrap();
    let mut rows_owner = RowIterator::new().unwrap();
    let mut cells_owner = CellIterator::new().unwrap();
    let mut rows = rows_owner.update(&snap).unwrap();
    let mut out = Vec::new();
    let mut y = 0;
    while let Some(row) = rows.next() {
        let mut cells = cells_owner.update(row).unwrap();
        let mut x = 0;
        let mut prev: Option<(u32, u32)> = None;
        while let Some(cell) = cells.next() {
            let g = cell.graphemes().unwrap_or_default();
            if g.first() == Some(&PLACEHOLDER) {
                let r = g.get(1).and_then(|&c| diacritic_index(c)).or(prev.map(|p| p.0)).unwrap_or(0);
                let c = g.get(2).and_then(|&c| diacritic_index(c)).or(prev.map(|p| p.1 + 1)).unwrap_or(0);
                out.push((y, x, r, c));
                prev = Some((r, c));
            } else {
                prev = None;
            }
            x += 1;
        }
        y += 1;
    }
    out
}

/// Assert that the placeholder cells form whole images: each run of cells shows consecutive
/// image columns starting at 0, and the image rows count up from 0 down the screen.
pub fn assert_whole_images(t: &Terminal<'_, '_>, placed: &[Placed]) {
    let cells = placeholder_positions(t);
    let total: u32 = placed.iter().map(|p| p.grid.0 * p.grid.1).sum();
    assert_eq!(cells.len() as u32, total, "placeholder cells vs. image grids {placed:?}");
    let mut by_row: std::collections::BTreeMap<usize, Vec<(usize, u32, u32)>> = Default::default();
    for (y, x, r, c) in cells {
        by_row.entry(y).or_default().push((x, r, c));
    }
    for (y, cells) in by_row {
        // Split into runs of adjacent screen cells (one run per image on this row; adjacent
        // images split where the image column starts over at 0).
        let mut start = 0;
        for i in 1..=cells.len() {
            if i == cells.len() || cells[i].0 != cells[i - 1].0 + 1 || cells[i].2 == 0 {
                let run = &cells[start..i];
                let cols: Vec<u32> = run.iter().map(|c| c.2).collect();
                let expected: Vec<u32> = (0..run.len() as u32).collect();
                assert_eq!(cols, expected, "screen row {y}: image columns of the run at x={}", run[0].0);
                assert!(
                    placed.iter().any(|p| p.grid.0 as usize == run.len()),
                    "screen row {y}: a run of {} cells at x={} doesn't span a whole image {placed:?}",
                    run.len(),
                    run[0].0
                );
                start = i;
            }
        }
    }
}

/// The pixels of kitty image `id` (RGBA, as the terminal stored them).
pub fn image_rgba(t: &Terminal<'_, '_>, id: u32) -> Option<(u32, u32, Vec<u8>)> {
    let g = t.kitty_graphics().ok()?;
    let img = g.image(id)?;
    Some((img.width().ok()?, img.height().ok()?, img.data().ok()?.to_vec()))
}

/// Pixels within `tol` of `rgb` (opaque or not) in an RGBA buffer.
pub fn count_color(rgba: &[u8], rgb: [u8; 3], tol: i32) -> usize {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0 && (0..3).all(|i| (i32::from(p[i]) - i32::from(rgb[i])).abs() <= tol))
        .count()
}
