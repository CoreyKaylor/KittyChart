//! The real-terminal adapter: the only I/O in this crate.
//!
//! It opens the controlling terminal (so it works even when stdout is piped), reads the window
//! size, and runs [`Detector`] queries in raw mode with a timeout. Raw mode is always restored,
//! including on early return.
//!
//! The platform layer is small: open, window size, a raw-mode guard, reads that return after
//! about 100 ms without input, and flushing unread input. Unix uses `/dev/tty` and termios;
//! Windows uses the console with virtual-terminal input and output (unverified: built and
//! linted, not yet run on Windows).

use std::io;
use std::time::{Duration, Instant};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix as sys;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as sys;

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
    port: sys::Port,
}

/// After the main timeout, how long to keep consuming late replies so they don't leak into the
/// shell as typed input.
const DRAIN: Duration = Duration::from_millis(300);

impl Tty {
    /// Open the controlling terminal.
    pub fn open() -> io::Result<Tty> {
        Ok(Tty { port: sys::Port::open()? })
    }

    pub fn window_size(&self) -> io::Result<WindowSize> {
        self.port.window_size()
    }

    /// Query the terminal's capabilities. Inside tmux (`$TMUX` set), the queries go to the
    /// outer terminal through passthrough (see [`Detector::in_tmux`]).
    pub fn query(&self, timeout: Duration) -> io::Result<Capabilities> {
        let d = if in_tmux() { Detector::new().in_tmux() } else { Detector::new() };
        self.query_with(d, timeout)
    }

    /// [`Tty::query`] with a configured detector. Returns once the DA1 sentinel arrives, or
    /// with whatever was learned when `timeout` elapses. Replies that arrive late are drained
    /// (and the input queue flushed) before cooked mode is restored, so they can't reach the
    /// shell.
    pub fn query_with(&self, mut detector: Detector, timeout: Duration) -> io::Result<Capabilities> {
        let _raw = self.port.raw()?;
        self.port.write_all(&detector.queries())?;
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
                self.port.flush_input();
                return Ok(detector.finish_timeout());
            }
            let n = self.port.read(&mut buf)?;
            if n == 0 {
                continue; // the read timed out with no input; check the clock again
            }
            // The sentinel can also arrive during the drain (slow replies). Either way it's complete.
            if let Some(caps) = detector.feed(&buf[..n]) {
                return Ok(caps.clone());
            }
        }
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.port.write_all(bytes)
    }

    /// Which out-of-band media the terminal accepts (see [`crate::local`]): sends a one-pixel
    /// query through each and reads the replies up to a DA1 sentinel. Inside tmux the queries
    /// go through passthrough. Anything the terminal didn't consume is removed.
    #[cfg(unix)]
    pub fn probe_media(&self, timeout: Duration, tmux: bool) -> io::Result<Media> {
        use crate::kitty::Medium;
        let pixel = [0u8, 0, 0, 255];
        let probes: Vec<(u32, Medium)> =
            [(0x4B44, crate::local::temp_file(&pixel)), (0x4B45, crate::local::shared_memory(&pixel))]
                .into_iter()
                .filter_map(|(id, m)| m.ok().map(|m| (id, m)))
                .collect();
        let mut q = Vec::new();
        for (id, m) in &probes {
            let (t, name) = match m {
                Medium::TempFile(p) => ('t', p),
                Medium::SharedMemory(n) => ('s', n),
                Medium::Direct => continue,
            };
            use base64::Engine;
            let b64 = base64::engine::general_purpose::STANDARD.encode(name.as_bytes());
            q.extend_from_slice(format!("\x1b_Gi={id},s=1,v=1,a=q,t={t},f=32,S=4;{b64}\x1b\\").as_bytes());
        }
        q.extend_from_slice(b"\x1b[c");
        let q = if tmux { crate::kitty::tmux_passthrough(&q) } else { q };

        let _raw = self.port.raw()?;
        self.port.write_all(&q)?;
        let deadline = Instant::now() + timeout;
        let mut replies = Vec::new();
        let mut buf = [0u8; 1024];
        while Instant::now() < deadline && !has_da1(&replies) {
            let n = self.port.read(&mut buf)?;
            replies.extend_from_slice(&buf[..n]);
        }
        self.port.flush_input();
        let text = String::from_utf8_lossy(&replies);
        let ok = |id: u32| text.contains(&format!("\x1b_Gi={id};OK"));
        let mut media = Media::default();
        for (id, m) in &probes {
            match m {
                Medium::TempFile(_) => media.temp_file = ok(*id),
                Medium::SharedMemory(_) => media.shared_memory = ok(*id),
                Medium::Direct => {}
            }
            if !crate::local::consumed(m) {
                crate::local::discard(m);
            }
        }
        Ok(media)
    }

    /// Out-of-band media are Unix-only for now: everything goes inline.
    #[cfg(not(unix))]
    pub fn probe_media(&self, _timeout: Duration, _tmux: bool) -> io::Result<Media> {
        Ok(Media::default())
    }

    /// Enter interactive mode: raw input, the alternate screen, mouse (SGR, pixel
    /// coordinates), focus, bracketed paste and resize reports (see [`crate::input::ENABLE`]).
    /// Everything is undone when the returned guard drops, including on panic.
    pub fn interactive(&self) -> io::Result<Interactive<'_>> {
        let raw = self.port.raw()?;
        self.port.write_all(crate::input::ENABLE.as_bytes())?;
        Ok(Interactive { tty: self, _raw: raw })
    }
}

/// Interactive mode (see [`Tty::interactive`]). Reads return within about 100 ms.
#[derive(Debug)]
pub struct Interactive<'a> {
    tty: &'a Tty,
    _raw: sys::Raw<'a>,
}

impl Interactive<'_> {
    /// Read available input into `buf`: `Ok(0)` after about 100 ms without input.
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.tty.port.read(buf)
    }

    /// Write and flush.
    pub fn write_all(&self, bytes: &[u8]) -> io::Result<()> {
        self.tty.port.write_all(bytes)
    }

    /// How long [`Interactive::read`] waits for input: zero returns at once (for animation
    /// frames), otherwise about `ms` (on Unix, rounded up to the driver's 100 ms steps).
    pub fn set_read_timeout(&self, ms: u32) -> io::Result<()> {
        self.tty.port.set_read_timeout(ms)
    }

    /// The window size now (polled: covers terminals without in-band resize reports).
    pub fn window_size(&self) -> io::Result<WindowSize> {
        self.tty.window_size()
    }
}

impl Drop for Interactive<'_> {
    fn drop(&mut self) {
        // Modes first, while still raw, then the guard restores the line discipline.
        let _ = self.tty.port.write_all(crate::input::DISABLE.as_bytes());
    }
}

/// Out-of-band media the terminal accepts (from [`Tty::probe_media`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Media {
    pub temp_file: bool,
    pub shared_memory: bool,
}

impl Media {
    /// The fastest supported medium: shared memory, then a temp file, then direct.
    pub fn best(&self) -> crate::kitty::Medium {
        use crate::kitty::Medium;
        if self.shared_memory {
            Medium::SharedMemory(String::new())
        } else if self.temp_file {
            Medium::TempFile(String::new())
        } else {
            Medium::Direct
        }
    }
}

#[cfg_attr(not(unix), allow(dead_code))]
fn has_da1(bytes: &[u8]) -> bool {
    bytes.windows(3).enumerate().any(|(i, w)| w == b"\x1b[?" && bytes[i..].iter().skip(3).any(|&c| c == b'c'))
}

/// UTF-16 code units (the Windows console's key characters) to UTF-8. A high surrogate at the
/// end waits in `high` for its low half, which may come with the next read; unpaired halves
/// become U+FFFD. Platform-independent so it's tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn decode_utf16(units: impl IntoIterator<Item = u16>, high: &mut Option<u16>, out: &mut Vec<u8>) {
    let mut push = |c: char| out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    for u in units {
        match u {
            0xD800..=0xDBFF => {
                if high.replace(u).is_some() {
                    push(char::REPLACEMENT_CHARACTER);
                }
            }
            0xDC00..=0xDFFF => match high.take() {
                Some(h) => {
                    let c = 0x10000 + ((u32::from(h) - 0xD800) << 10) + (u32::from(u) - 0xDC00);
                    push(char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                None => push(char::REPLACEMENT_CHARACTER),
            },
            _ => {
                if high.take().is_some() {
                    push(char::REPLACEMENT_CHARACTER);
                }
                push(char::from_u32(u32::from(u)).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }
}

/// Whether this process runs inside tmux. Graphics must then be wrapped for passthrough and
/// drawn with Unicode placeholders.
pub fn in_tmux() -> bool {
    std::env::var_os("TMUX").is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::decode_utf16;

    fn decode(chunks: &[&[u16]]) -> String {
        let (mut high, mut out) = (None, Vec::new());
        for c in chunks {
            decode_utf16(c.iter().copied(), &mut high, &mut out);
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn utf16_escape_sequences_and_text() {
        let reply: Vec<u16> = "\x1b[?62;4c é".encode_utf16().collect();
        assert_eq!(decode(&[&reply]), "\x1b[?62;4c é");
    }

    #[test]
    fn utf16_surrogate_pair_split_across_reads() {
        let units: Vec<u16> = "a📈b".encode_utf16().collect();
        assert_eq!(units.len(), 4);
        assert_eq!(decode(&[&units[..2], &units[2..]]), "a📈b");
    }

    #[test]
    fn utf16_unpaired_surrogates_become_replacements() {
        assert_eq!(decode(&[&[0xD83D, 0x41]]), "\u{FFFD}A");
        assert_eq!(decode(&[&[0xDC00]]), "\u{FFFD}");
        assert_eq!(decode(&[&[0xD83D, 0xD83D, 0xDCC8]]), "\u{FFFD}📈");
    }
}
