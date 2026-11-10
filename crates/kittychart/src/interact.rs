//! A full-screen interactive chart in the user's terminal: hover for tooltips, click legend
//! items, arrow keys, `r` replay, `s` save, `q` quit; optionally following a file. The loop
//! behind `kittychart view`, Python's `Chart.interact()` and Node's `chart.interact()`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use kittychart_term::input::{Event, Parser};
use kittychart_term::kitty::Medium;
use kittychart_term::tty::{Interactive, Tty};

use crate::session::{Action, Session, SessionOptions};
use crate::show::{Mode, PROBE_TIMEOUT, ShowError, probe};
use crate::{ChartConfig, Color};

/// Options for [`interact`].
#[derive(Debug, Clone, Default)]
pub struct InteractOptions {
    /// `None`: `$KITTYCHART_PROTOCOL`, else the best the terminal supports.
    pub mode: Option<Mode>,
    pub scale: Option<f32>,
    pub background: Option<Color>,
    pub chartjs_colors: bool,
    /// kitty: always send frames inline, even when the terminal reads temp files or shared
    /// memory.
    pub no_fast_transfer: bool,
    /// Follow this file: re-read it when it changes and animate to its contents.
    pub watch: Option<PathBuf>,
    /// Where `s` saves PNGs (default: the current directory).
    pub save_dir: Option<PathBuf>,
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Ask whether the terminal accepted pixel mouse reports (DECRQM 1016).
fn pixel_mouse(term: &Interactive<'_>) -> bool {
    if term.write_all(b"\x1b[?1016$p").is_err() {
        return false;
    }
    let deadline = Instant::now() + Duration::from_millis(300);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while Instant::now() < deadline {
        let n = term.read(&mut buf).unwrap_or(0);
        got.extend_from_slice(&buf[..n]);
        let s = String::from_utf8_lossy(&got);
        if let Some(i) = s.find("\x1b[?1016;") {
            // Ps: 1 set, 2 reset, 3 permanently set, 4 permanently reset, 0 unknown.
            return matches!(s[i + 8..].chars().next(), Some('1' | '3'));
        }
    }
    false
}

/// Run the interactive viewer until the user quits. `reload` is called with `opts.watch`'s
/// contents when that file changes (return the new config, or an error to show).
pub fn interact(
    config: ChartConfig,
    opts: &InteractOptions,
    mut reload: impl FnMut(&str) -> Result<ChartConfig, String>,
) -> Result<(), ShowError> {
    let mode = match opts.mode {
        Some(m) => m,
        None => Mode::from_env()?.unwrap_or_default(),
    };
    let info = probe(PROBE_TIMEOUT).map_err(|e| ShowError::Usage(format!("no terminal ({e})")))?;
    let tty = Tty::open()?;
    let mode = if mode == Mode::Auto { info.best_mode() } else { mode };
    let medium = if mode == Mode::Kitty && !opts.no_fast_transfer {
        tty.probe_media(PROBE_TIMEOUT, info.tmux).map(|m| m.best()).unwrap_or_default()
    } else {
        Medium::Direct
    };
    let sopts = SessionOptions {
        mode,
        scale: opts.scale,
        background: opts.background,
        chartjs_colors: opts.chartjs_colors,
        medium,
        ..SessionOptions::default()
    };
    let mut session = Session::new(config, info, sopts);

    let term = tty.interactive()?;
    let mut parser = Parser::new(pixel_mouse(&term));
    let start = Instant::now();
    let mut last_mtime = opts.watch.as_deref().and_then(mtime);
    let mut last_poll = Instant::now();
    let mut last_size = term.window_size().ok();
    let mut buf = [0u8; 4096];
    let mut status: Option<(String, Instant)> = None;
    'main: loop {
        // Wait for input briefly when idle; don't when an animation needs frames.
        let _ = term.set_read_timeout(if session.needs_frame() { 0 } else { 100 });
        let n = term.read(&mut buf)?;
        let mut events = parser.feed(&buf[..n]);
        if n == 0 {
            events.extend(parser.flush());
            if session.needs_frame() {
                std::thread::sleep(Duration::from_millis(4));
            }
        }
        for ev in &events {
            match session.handle(ev) {
                Action::Quit => break 'main,
                Action::Save(png) => {
                    let secs =
                        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                    let dir = opts.save_dir.clone().unwrap_or_default();
                    let path = dir.join(format!("kittychart-{secs}.png"));
                    let msg = match std::fs::write(&path, png) {
                        Ok(()) => format!("saved {}", path.display()),
                        Err(e) => format!("{}: {e}", path.display()),
                    };
                    status = Some((msg, Instant::now()));
                }
                Action::Redraw | Action::None => {}
            }
            if let Event::Resize { .. } = ev {
                last_size = term.window_size().ok();
            }
        }
        if last_poll.elapsed() >= Duration::from_millis(250) {
            last_poll = Instant::now();
            // Resize (terminals without in-band reports).
            if let Ok(ws) = term.window_size()
                && Some(ws) != last_size
            {
                last_size = Some(ws);
                let (cw, ch) = ws.cell_px().map_or((10.0, 20.0), |(w, h)| (f32::from(w), f32::from(h)));
                session.resize(ws.cols, ws.rows, cw, ch);
                term.write_all(b"\x1b[2J")?;
            }
            if let Some(path) = &opts.watch {
                let m = mtime(path);
                if m != last_mtime {
                    last_mtime = m;
                    let next = std::fs::read_to_string(path)
                        .map_err(|e| format!("{}: {e}", path.display()))
                        .and_then(|json| reload(&json));
                    match next {
                        Ok(c) => session.update(c),
                        Err(e) => status = Some((e, Instant::now())),
                    }
                }
            }
        }
        let now = start.elapsed().as_secs_f64() * 1000.0;
        if let Some(bytes) = session.frame(now) {
            term.write_all(&bytes)?;
        }
        if let Some((msg, at)) = &status {
            if at.elapsed() < Duration::from_secs(3) {
                let ws = term.window_size()?;
                let line = format!("\x1b7\x1b[{};1H\x1b[2K\x1b[7m {msg} \x1b[0m\x1b8", ws.rows);
                term.write_all(line.as_bytes())?;
            } else {
                status = None;
                session.redraw();
            }
        }
    }
    term.write_all(&session.cleanup())?;
    Ok(())
}
