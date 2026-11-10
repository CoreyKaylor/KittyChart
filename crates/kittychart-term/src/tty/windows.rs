//! The Windows console: `CONIN$`/`CONOUT$` (so it works even when stdout is redirected) with
//! virtual-terminal input and output, which is how Windows Terminal, WezTerm and others pass
//! escape sequences (query replies, mouse reports) through the console API.
//!
//! Reads wait on the input handle with a timeout, then take the console's input records and
//! keep the characters of key-down events: with `ENABLE_VIRTUAL_TERMINAL_INPUT`, terminal
//! replies arrive as those characters (UTF-16, decoded by [`super::decode_utf16`]). Other
//! records (focus, buffer size, menu) are consumed and ignored, so a wait never blocks on them.
//!
//! Unverified: cross-compiled and linted on Linux, not yet run on Windows.

#![allow(unsafe_code)] // Win32 console calls; each block passes valid handles and buffers.

use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::io::AsRawHandle;

use windows_sys::Win32::Foundation::{HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS, ENABLE_LINE_INPUT,
    ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE,
    ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
    FlushConsoleInputBuffer, GetConsoleMode, GetConsoleScreenBufferInfo, GetNumberOfConsoleInputEvents,
    INPUT_RECORD, KEY_EVENT, ReadConsoleInputW, SetConsoleMode,
};
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use super::WindowSize;

/// The default wait for input, like the Unix driver's `VTIME = 1`.
const DEFAULT_TIMEOUT_MS: u32 = 100;

#[derive(Debug)]
pub(super) struct Port {
    input: File,
    output: File,
    timeout_ms: Cell<u32>,
    /// A high surrogate whose low half hasn't arrived yet.
    high: Cell<Option<u16>>,
    /// Decoded bytes that didn't fit the caller's buffer.
    pending: RefCell<Vec<u8>>,
}

fn check(ok: i32) -> io::Result<()> {
    if ok == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

fn handle(f: &File) -> HANDLE {
    f.as_raw_handle() as HANDLE
}

fn mode(f: &File) -> io::Result<CONSOLE_MODE> {
    let mut m: CONSOLE_MODE = 0;
    // SAFETY: a console handle we own and a valid out pointer.
    check(unsafe { GetConsoleMode(handle(f), &mut m) })?;
    Ok(m)
}

fn set_mode(f: &File, m: CONSOLE_MODE) -> io::Result<()> {
    // SAFETY: a console handle we own.
    check(unsafe { SetConsoleMode(handle(f), m) })
}

impl Port {
    pub(super) fn open() -> io::Result<Port> {
        Ok(Port {
            input: OpenOptions::new().read(true).write(true).open("CONIN$")?,
            output: OpenOptions::new().read(true).write(true).open("CONOUT$")?,
            timeout_ms: Cell::new(DEFAULT_TIMEOUT_MS),
            high: Cell::new(None),
            pending: RefCell::new(Vec::new()),
        })
    }

    /// The visible window in cells. The console API has no pixel size; callers fall back to
    /// the terminal's own answer (`CSI 16 t`, see [`crate::detect`]).
    pub(super) fn window_size(&self) -> io::Result<WindowSize> {
        // SAFETY: CONSOLE_SCREEN_BUFFER_INFO is plain data; zeroed is a valid value.
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: a console output handle we own and a valid out pointer.
        check(unsafe { GetConsoleScreenBufferInfo(handle(&self.output), &mut info) })?;
        let w = info.srWindow;
        let span = |a: i16, b: i16| u16::try_from(i32::from(b) - i32::from(a) + 1).unwrap_or(0);
        Ok(WindowSize { cols: span(w.Left, w.Right), rows: span(w.Top, w.Bottom), px_width: 0, px_height: 0 })
    }

    /// Raw, virtual-terminal input and output with a 100 ms read timeout, until the guard drops.
    pub(super) fn raw(&self) -> io::Result<Raw<'_>> {
        let (saved_in, saved_out) = (mode(&self.input)?, mode(&self.output)?);
        let cooked = ENABLE_LINE_INPUT
            | ENABLE_ECHO_INPUT
            | ENABLE_PROCESSED_INPUT
            | ENABLE_MOUSE_INPUT
            | ENABLE_WINDOW_INPUT
            | ENABLE_QUICK_EDIT_MODE;
        set_mode(&self.input, (saved_in & !cooked) | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_EXTENDED_FLAGS)?;
        let raw = Raw { port: self, saved_in, saved_out };
        set_mode(&self.output, saved_out | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING)?;
        self.timeout_ms.set(DEFAULT_TIMEOUT_MS);
        Ok(raw)
    }

    /// Read input: `Ok(0)` when the timeout passes without any characters.
    pub(super) fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pending.borrow().is_empty() {
            self.fill()?;
        }
        let mut pending = self.pending.borrow_mut();
        let n = pending.len().min(buf.len());
        buf[..n].copy_from_slice(&pending[..n]);
        pending.drain(..n);
        Ok(n)
    }

    /// Wait for input records, then decode the characters of the key-down events.
    fn fill(&self) -> io::Result<()> {
        let h = handle(&self.input);
        // SAFETY: a console input handle we own.
        if unsafe { WaitForSingleObject(h, self.timeout_ms.get()) } != WAIT_OBJECT_0 {
            return Ok(());
        }
        let mut available = 0u32;
        // SAFETY: as above, with a valid out pointer.
        check(unsafe { GetNumberOfConsoleInputEvents(h, &mut available) })?;
        if available == 0 {
            return Ok(());
        }
        // SAFETY: INPUT_RECORD is plain data; zeroed is a valid value.
        let mut records: Vec<INPUT_RECORD> = vec![unsafe { std::mem::zeroed() }; available.min(512) as usize];
        let mut read = 0u32;
        // SAFETY: `records` has room for the count we pass; `read` is a valid out pointer.
        check(unsafe { ReadConsoleInputW(h, records.as_mut_ptr(), records.len() as u32, &mut read) })?;
        let units =
            records[..read as usize].iter().filter(|r| u32::from(r.EventType) == KEY_EVENT).flat_map(|r| {
                // SAFETY: EventType says this record holds a key event.
                let k = unsafe { r.Event.KeyEvent };
                // SAFETY: both union members are plain integers; UnicodeChar is the W variant's.
                let unit = unsafe { k.uChar.UnicodeChar };
                let n = if k.bKeyDown != 0 && unit != 0 { k.wRepeatCount.max(1) } else { 0 };
                std::iter::repeat_n(unit, usize::from(n))
            });
        let mut high = self.high.get();
        super::decode_utf16(units, &mut high, &mut self.pending.borrow_mut());
        self.high.set(high);
        Ok(())
    }

    pub(super) fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        let mut f = &self.output;
        f.write_all(bytes)?;
        f.flush()
    }

    /// Drop input that hasn't been read (late query replies).
    pub(super) fn flush_input(&self) {
        self.pending.borrow_mut().clear();
        self.high.set(None);
        // SAFETY: a console input handle we own.
        unsafe { FlushConsoleInputBuffer(handle(&self.input)) };
    }

    /// How long [`Port::read`] waits; zero returns at once.
    pub(super) fn set_read_timeout(&self, ms: u32) -> io::Result<()> {
        self.timeout_ms.set(ms);
        Ok(())
    }
}

/// Raw mode (see [`Port::raw`]); the console modes are restored on drop.
#[derive(Debug)]
pub(super) struct Raw<'a> {
    port: &'a Port,
    saved_in: CONSOLE_MODE,
    saved_out: CONSOLE_MODE,
}

impl Drop for Raw<'_> {
    fn drop(&mut self) {
        let _ = set_mode(&self.port.input, self.saved_in);
        let _ = set_mode(&self.port.output, self.saved_out);
        self.port.timeout_ms.set(DEFAULT_TIMEOUT_MS);
    }
}
