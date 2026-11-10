//! The Unix terminal: `/dev/tty` (so it works even when stdout is piped) in raw mode.
//!
//! The read timeout uses the terminal driver itself (raw mode with `VMIN = 0`, `VTIME = 1`:
//! each `read` returns after at most 100 ms). That works on every Unix, including macOS, where
//! `poll()` doesn't support tty devices.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};

use rustix::termios::{
    OptionalActions, QueueSelector, SpecialCodeIndex, Termios, tcflush, tcgetattr, tcgetwinsize, tcsetattr,
};

use super::WindowSize;

#[derive(Debug)]
pub(super) struct Port {
    file: File,
}

impl Port {
    pub(super) fn open() -> io::Result<Port> {
        Ok(Port { file: OpenOptions::new().read(true).write(true).open("/dev/tty")? })
    }

    pub(super) fn window_size(&self) -> io::Result<WindowSize> {
        let ws = tcgetwinsize(&self.file)?;
        Ok(WindowSize { cols: ws.ws_col, rows: ws.ws_row, px_width: ws.ws_xpixel, px_height: ws.ws_ypixel })
    }

    /// Raw mode with a 100 ms read timeout, until the guard drops.
    pub(super) fn raw(&self) -> io::Result<Raw<'_>> {
        let saved = tcgetattr(&self.file)?;
        let mut raw = saved.clone();
        raw.make_raw();
        raw.special_codes[SpecialCodeIndex::VMIN] = 0;
        raw.special_codes[SpecialCodeIndex::VTIME] = 1;
        tcsetattr(&self.file, OptionalActions::Now, &raw)?;
        Ok(Raw { file: &self.file, saved })
    }

    /// Read input in raw mode: `Ok(0)` when the timeout passes without any.
    pub(super) fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        match (&self.file).read(buf) {
            Err(e) if e.kind() == io::ErrorKind::Interrupted => Ok(0), // e.g. SIGWINCH
            r => r,
        }
    }

    pub(super) fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        let mut f = &self.file;
        f.write_all(bytes)?;
        f.flush()
    }

    /// Drop input that hasn't been read (late query replies).
    pub(super) fn flush_input(&self) {
        let _ = tcflush(&self.file, QueueSelector::IFlush);
    }

    /// Zero returns at once; otherwise about `ms`, in the driver's 100 ms steps.
    pub(super) fn set_read_timeout(&self, ms: u32) -> io::Result<()> {
        let mut t = tcgetattr(&self.file)?;
        t.special_codes[SpecialCodeIndex::VMIN] = 0;
        t.special_codes[SpecialCodeIndex::VTIME] = if ms == 0 { 0 } else { ms.div_ceil(100).min(255) as u8 };
        tcsetattr(&self.file, OptionalActions::Now, &t)?;
        Ok(())
    }
}

/// Raw mode (see [`Port::raw`]); the line discipline is restored on drop.
#[derive(Debug)]
pub(super) struct Raw<'a> {
    file: &'a File,
    saved: Termios,
}

impl Drop for Raw<'_> {
    fn drop(&mut self) {
        let _ = tcsetattr(self.file, OptionalActions::Now, &self.saved);
    }
}
