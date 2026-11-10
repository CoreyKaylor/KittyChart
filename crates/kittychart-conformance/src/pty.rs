//! Run a real program on a pseudo-terminal whose other end is Ghostty's VT core.
//!
//! The program sees a genuine TTY (raw mode, `TIOCGWINSZ` with pixel sizes, a controlling
//! terminal), and every query it sends is answered by the emulator: kitty graphics, DA1, cell
//! size, OSC colors. So this exercises the whole path a user hits, including the CLI's TTY
//! adapter, tmux in the middle, or a ratatui app, without opening a window.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use libghostty_vt::Terminal;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// Bytes to type at a given time after start.
#[derive(Debug, Clone)]
pub struct Key {
    pub at: Duration,
    pub bytes: Vec<u8>,
}

/// Parse `"1500:q,3000:\r"`-style key scripts. `\r`, `\n`, `\e` and `\s` (space) are escapes.
pub fn parse_keys(s: &str) -> Vec<Key> {
    s.split(',')
        .filter(|k| !k.is_empty())
        .map(|k| {
            let (ms, text) = k.split_once(':').expect("keys look like MS:TEXT");
            let text =
                text.replace("\\r", "\r").replace("\\n", "\n").replace("\\e", "\x1b").replace("\\s", " ");
            Key { at: Duration::from_millis(ms.parse().expect("key time in ms")), bytes: text.into_bytes() }
        })
        .collect()
}

#[derive(Debug)]
pub struct Outcome {
    /// Everything the program wrote, for protocol-level checks.
    pub output: Vec<u8>,
    /// The exit status, when it exited before the snapshot/timeout.
    pub exit: Option<u32>,
}

/// Run `argv` on a PTY sized `cols × rows` cells of `cw × ch` pixels, feeding its output into
/// `term` and the terminal's replies (`replies`, filled by the terminal's pty-write callback)
/// back to it. Stops at `snapshot` (killing the program) or when the program exits.
#[allow(clippy::too_many_arguments)]
pub fn run(
    term: &mut Terminal<'_, '_>,
    replies: &Rc<RefCell<Vec<u8>>>,
    argv: &[String],
    env: &[(String, String)],
    (cols, rows, cw, ch): (u16, u16, u32, u32),
    keys: &[Key],
    snapshot: Option<Duration>,
    timeout: Duration,
) -> Outcome {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: (u32::from(cols) * cw) as u16,
            pixel_height: (u32::from(rows) * ch) as u16,
        })
        .expect("openpty");
    // A relative program path is relative to the current directory, as in a shell.
    let program = if argv[0].contains('/') {
        std::fs::canonicalize(&argv[0]).unwrap_or_else(|e| panic!("{}: {e}", argv[0]))
    } else {
        argv[0].clone().into()
    };
    let mut cmd = CommandBuilder::new(program);
    cmd.args(&argv[1..]);
    cmd.cwd(std::env::current_dir().expect("cwd"));
    // Ghostty's own terminfo where installed; programs that read terminfo (tmux) need it.
    let ghostty_terminfo =
        std::process::Command::new("infocmp").arg("xterm-ghostty").output().is_ok_and(|o| o.status.success());
    cmd.env("TERM", if ghostty_terminfo { "xterm-ghostty" } else { "xterm-256color" });
    cmd.env("COLORTERM", "truecolor");
    cmd.env_remove("TMUX");
    cmd.env_remove("KITTYCHART_PROTOCOL");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut writer = pair.master.take_writer().expect("writer");

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut buf = [0u8; 65536];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF / EIO once the program and its children are gone
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let start = Instant::now();
    let trace = std::env::var_os("KC_PTY_TRACE").is_some();
    let mut output = Vec::new();
    let mut next_key = 0;
    let mut exit = None;
    let mut exited_at: Option<Instant> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(chunk) => {
                if trace {
                    eprintln!("[{:>6} ms] out {} bytes", start.elapsed().as_millis(), chunk.len());
                }
                output.extend_from_slice(&chunk);
                let t0 = Instant::now();
                term.vt_write(&chunk);
                if trace {
                    eprintln!("          vt_write took {} ms", t0.elapsed().as_millis());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let r = std::mem::take(&mut *replies.borrow_mut());
        if !r.is_empty() {
            if trace {
                eprintln!("[{:>6} ms] reply {:?}", start.elapsed().as_millis(), String::from_utf8_lossy(&r));
            }
            let _ = writer.write_all(&r).and_then(|()| writer.flush());
        }
        let now = start.elapsed();
        while next_key < keys.len() && keys[next_key].at <= now {
            let _ = writer.write_all(&keys[next_key].bytes).and_then(|()| writer.flush());
            next_key += 1;
        }
        if snapshot.is_some_and(|s| now >= s) {
            break;
        }
        if exit.is_none()
            && let Ok(Some(status)) = child.try_wait()
        {
            exit = Some(status.exit_code());
            exited_at = Some(Instant::now());
        }
        // After exit, drain briefly (output still in flight), then stop.
        if exited_at.is_some_and(|t| t.elapsed() > Duration::from_millis(150)) {
            break;
        }
        if now > timeout {
            eprintln!("pty: {argv:?} timed out after {timeout:?}");
            break;
        }
    }
    // EOF usually arrives just before the exit status is ready.
    let reaped = Instant::now();
    while exit.is_none() && snapshot.is_none() && reaped.elapsed() < Duration::from_millis(500) {
        match child.try_wait() {
            Ok(Some(status)) => exit = Some(status.exit_code()),
            _ => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    if exit.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    Outcome { output, exit }
}
