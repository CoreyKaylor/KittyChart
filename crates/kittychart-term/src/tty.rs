//! The real-terminal adapter: the only I/O in this crate.
//!
//! It opens `/dev/tty` (so it works even when stdout is piped), reads the window size, and runs
//! [`Detector`] queries in raw mode with a timeout. Raw mode is always restored, including on
//! early return.
//!
//! The read timeout uses the terminal driver itself (raw mode with `VMIN = 0`, `VTIME = 1`:
//! each `read` returns after at most 100 ms). That works on every Unix, including macOS, where
//! `poll()` doesn't support tty devices.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use rustix::termios::{
    OptionalActions, QueueSelector, SpecialCodeIndex, Termios, tcflush, tcgetattr, tcgetwinsize, tcsetattr,
};

use crate::detect::{Capabilities, Detector};

/// Terminal window size in cells and (when the terminal reports it) pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub cols: u16,
    pub rows: u16,
    pub px_width: u16,
    pub px_height: u16,
}

impl WindowSize {
    /// Cell size in pixels, if the terminal reports usable pixel dimensions.
    pub fn cell_px(&self) -> Option<(u16, u16)> {
        if self.cols == 0 || self.rows == 0 {
            return None;
        }
        let (w, h) = (self.px_width / self.cols, self.px_height / self.rows);
        (w > 0 && h > 0).then_some((w, h))
    }
}

#[derive(Debug)]
pub struct Tty {
    file: File,
}

/// After the main timeout, how long to keep consuming late replies so they don't leak into the
/// shell as typed input.
const DRAIN: Duration = Duration::from_millis(300);

impl Tty {
    /// Open the controlling terminal.
    pub fn open() -> io::Result<Tty> {
        Ok(Tty { file: OpenOptions::new().read(true).write(true).open("/dev/tty")? })
    }

    pub fn window_size(&self) -> io::Result<WindowSize> {
        let ws = tcgetwinsize(&self.file)?;
        Ok(WindowSize { cols: ws.ws_col, rows: ws.ws_row, px_width: ws.ws_xpixel, px_height: ws.ws_ypixel })
    }

    /// Ask the terminal what it supports. Returns once the DA1 sentinel arrives, or with whatever
    /// was learned when `timeout` elapses. Replies that arrive late are drained (and the input
    /// queue flushed) before cooked mode is restored, so they can't reach the shell.
    pub fn query(&self, timeout: Duration) -> io::Result<Capabilities> {
        let _raw = RawMode::enable(&self.file)?;
        let mut f = &self.file; // `&File` implements Read + Write
        let mut detector = Detector::new();
        f.write_all(&detector.queries())?;
        f.flush()?;
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 1024];
        let mut timed_out_caps = None;
        loop {
            let now = Instant::now();
            if timed_out_caps.is_none() && now >= deadline {
                // Keep listening briefly for the sentinel so stragglers are consumed.
                timed_out_caps = Some(deadline + DRAIN);
            }
            if let Some(drain_until) = timed_out_caps
                && now >= drain_until
            {
                let _ = tcflush(&self.file, QueueSelector::IFlush);
                return Ok(detector.finish_timeout());
            }
            let n = match f.read(&mut buf) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue, // e.g. SIGWINCH
                Err(e) => return Err(e),
            };
            if n == 0 {
                continue; // VTIME expired with no input; check the clock again
            }
            // The sentinel can also arrive during the drain (slow replies). Either way it's complete.
            if let Some(caps) = detector.feed(&buf[..n]) {
                return Ok(caps.clone());
            }
        }
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file.write_all(bytes)?;
        self.file.flush()
    }
}

/// Raw mode with a 100 ms read timeout, for the lifetime of the guard.
struct RawMode<'a> {
    file: &'a File,
    saved: Termios,
}

impl<'a> RawMode<'a> {
    fn enable(file: &'a File) -> io::Result<Self> {
        let saved = tcgetattr(file)?;
        let mut raw = saved.clone();
        raw.make_raw();
        raw.special_codes[SpecialCodeIndex::VMIN] = 0;
        raw.special_codes[SpecialCodeIndex::VTIME] = 1;
        tcsetattr(file, OptionalActions::Now, &raw)?;
        Ok(RawMode { file, saved })
    }
}

impl Drop for RawMode<'_> {
    fn drop(&mut self) {
        let _ = tcsetattr(self.file, OptionalActions::Now, &self.saved);
    }
}
