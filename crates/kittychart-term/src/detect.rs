//! Sans-IO terminal capability detection.
//!
//! Usage: write [`Detector::queries`] to the terminal, then pass every byte read back into
//! [`Detector::feed`] until it returns capabilities, or until your timeout fires
//! ([`Detector::finish_timeout`]). Terminals answer queries in order, so the DA1 reply, which
//! every terminal sends, works as a sentinel. Once it arrives, all earlier replies have arrived.
//!
//! The real TTY loop (raw mode, poll with timeout) belongs to the host adapter. This type never
//! touches I/O, so tests can drive it with byte slices or with a real emulator's replies.

use crate::kitty;

/// Image id used for the kitty support probe. Arbitrary, but unlikely to collide with app ids.
pub const KITTY_PROBE_ID: u32 = 0x4B43; // "KC"

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// The terminal answered the kitty graphics `a=q` probe with OK.
    pub kitty_graphics: bool,
    /// DA1 advertised attribute 4 (sixel graphics).
    pub sixel: bool,
    /// Cell size in pixels from `CSI 16 t`, as `(width, height)`.
    pub cell_px: Option<(u16, u16)>,
    /// Raw DA1 attributes, kept for diagnostics.
    pub da1: Vec<u16>,
    /// Default foreground (OSC 10) and background (OSC 11), as 8-bit RGB.
    pub foreground: Option<[u8; 3]>,
    pub background: Option<[u8; 3]>,
    /// ANSI palette 0–15 (OSC 4). Entries the terminal didn't report are `None`.
    pub palette: [Option<[u8; 3]>; 16],
}

impl Capabilities {
    /// The full 16-color palette, if every entry was reported.
    pub fn full_palette(&self) -> Option<[[u8; 3]; 16]> {
        let mut out = [[0; 3]; 16];
        for (o, p) in out.iter_mut().zip(self.palette) {
            *o = p?;
        }
        Some(out)
    }
}

#[derive(Debug, Default)]
pub struct Detector {
    buf: Vec<u8>,
    caps: Capabilities,
    done: bool,
}

impl Detector {
    pub fn new() -> Self {
        Self::default()
    }

    /// The bytes to send to the terminal: kitty probe, cell-size report, then the DA1 sentinel.
    pub fn queries(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        kitty::support_query(KITTY_PROBE_ID, &mut out);
        out.extend_from_slice(b"\x1b[16t"); // report cell size in pixels
        out.extend_from_slice(b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\"); // default fg / bg
        for i in 0..16 {
            out.extend_from_slice(format!("\x1b]4;{i};?\x1b\\").as_bytes()); // palette
        }
        out.extend_from_slice(b"\x1b[c"); // DA1: must be last
        out
    }

    /// Feed bytes read from the terminal. Returns the capabilities once the DA1 sentinel is seen.
    /// Bytes that are not query replies (such as user keystrokes) are ignored.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<&Capabilities> {
        if self.done {
            return Some(&self.caps);
        }
        self.buf.extend_from_slice(bytes);
        let mut i = 0;
        while let Some(rel) = self.buf[i..].iter().position(|&b| b == 0x1b) {
            let start = i + rel;
            match self.parse_at(start) {
                Parse::Complete(len) => i = start + len,
                Parse::Incomplete => break,
                Parse::Skip => i = start + 1,
            }
            if self.done {
                break;
            }
        }
        // Drop consumed bytes, but keep an incomplete trailing sequence for the next call.
        let keep_from = self.buf[i..].iter().position(|&b| b == 0x1b).map_or(self.buf.len(), |r| i + r);
        self.buf.drain(..keep_from);
        self.done.then_some(&self.caps)
    }

    /// Call when the host's timeout expires with no DA1 reply. The terminal is assumed to
    /// support nothing beyond what was already seen.
    pub fn finish_timeout(self) -> Capabilities {
        self.caps
    }

    fn parse_at(&mut self, s: usize) -> Parse {
        let b = &self.buf[s..];
        if b.len() < 2 {
            return Parse::Incomplete;
        }
        match b[1] {
            b'_' => {
                // APC ... ST
                let Some(end) = find_st(b) else { return Parse::Incomplete };
                let body = &b[2..end];
                if let Some(rest) = body.strip_prefix(b"G") {
                    let (keys, msg) = split_once(rest, b';');
                    let probe = format!("i={KITTY_PROBE_ID}");
                    if keys.split(|&c| c == b',').any(|k| k == probe.as_bytes()) && msg == b"OK" {
                        self.caps.kitty_graphics = true;
                    }
                }
                Parse::Complete(end + 2)
            }
            b']' => {
                // OSC ... (ST | BEL)
                let body = &b[2..];
                let end = body.iter().enumerate().find_map(|(i, &c)| match c {
                    0x07 => Some((i, 1)),
                    0x1b if body.get(i + 1) == Some(&b'\\') => Some((i, 2)),
                    _ => None,
                });
                let Some((end, term_len)) = end else { return Parse::Incomplete };
                let osc = body[..end].to_vec();
                self.parse_osc(&osc);
                Parse::Complete(2 + end + term_len)
            }
            b'[' => {
                // CSI params final
                let Some(fin) = b[2..].iter().position(|&c| (0x40..=0x7e).contains(&c)) else {
                    return Parse::Incomplete;
                };
                let params = &b[2..2 + fin];
                match b[2 + fin] {
                    b'c' if params.first() == Some(&b'?') => {
                        self.caps.da1 = parse_nums(&params[1..]);
                        self.caps.sixel = self.caps.da1.contains(&4);
                        self.done = true;
                    }
                    b't' => {
                        if let [6, h, w] = parse_nums(params)[..] {
                            self.caps.cell_px = Some((w, h));
                        }
                    }
                    _ => {}
                }
                Parse::Complete(2 + fin + 1)
            }
            _ => Parse::Skip,
        }
    }
}

impl Detector {
    fn parse_osc(&mut self, body: &[u8]) {
        let Ok(s) = std::str::from_utf8(body) else { return };
        let mut parts = s.split(';');
        match parts.next() {
            Some("10") => self.caps.foreground = parts.next().and_then(parse_xcolor),
            Some("11") => self.caps.background = parts.next().and_then(parse_xcolor),
            Some("4") => {
                if let (Some(Ok(i)), Some(c)) = (parts.next().map(str::parse::<usize>), parts.next())
                    && i < 16
                {
                    self.caps.palette[i] = parse_xcolor(c);
                }
            }
            _ => {}
        }
    }
}

/// X11 color spec `rgb:R/G/B` with 1–4 hex digits per channel, or `#RRGGBB`.
fn parse_xcolor(s: &str) -> Option<[u8; 3]> {
    if let Some(rest) = s.strip_prefix("rgb:") {
        let mut it = rest.split('/');
        let mut out = [0u8; 3];
        for o in &mut out {
            let h = it.next()?;
            if h.is_empty() || h.len() > 4 {
                return None;
            }
            let v = u32::from_str_radix(h, 16).ok()?;
            let max = (1u32 << (4 * h.len())) - 1;
            *o = ((v * 255 + max / 2) / max) as u8;
        }
        return Some(out);
    }
    let h = s.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

enum Parse {
    Complete(usize),
    Incomplete,
    Skip,
}

fn find_st(b: &[u8]) -> Option<usize> {
    b.windows(2).position(|w| w == b"\x1b\\").filter(|&p| p >= 2)
}

fn split_once(b: &[u8], sep: u8) -> (&[u8], &[u8]) {
    match b.iter().position(|&c| c == sep) {
        Some(p) => (&b[..p], &b[p + 1..]),
        None => (b, &[]),
    }
}

fn parse_nums(b: &[u8]) -> Vec<u16> {
    b.split(|&c| c == b';').filter_map(|n| std::str::from_utf8(n).ok()?.parse().ok()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITTY_OK: &[u8] = b"\x1b_Gi=19267;OK\x1b\\";

    #[test]
    fn kitty_like_terminal() {
        let mut d = Detector::new();
        let caps = d.feed(&[KITTY_OK, b"\x1b[6;20;10t\x1b[?62;22c"].concat()).cloned().unwrap();
        assert!(caps.kitty_graphics);
        assert!(!caps.sixel);
        assert_eq!(caps.cell_px, Some((10, 20)));
    }

    #[test]
    fn sixel_only_terminal_split_across_reads() {
        let mut d = Detector::new();
        assert!(d.feed(b"\x1b[?62;4").is_none());
        let caps = d.feed(b";22c").unwrap();
        assert!(!caps.kitty_graphics);
        assert!(caps.sixel);
    }

    #[test]
    fn byte_at_a_time_with_noise() {
        let input = [b"x".as_slice(), KITTY_OK, b"\x1b[?1;2c"].concat();
        let mut d = Detector::new();
        let mut result = None;
        for b in input {
            if let Some(c) = d.feed(&[b]) {
                result = Some(c.clone());
            }
        }
        assert!(result.unwrap().kitty_graphics);
    }

    #[test]
    fn osc_colors_with_both_terminators() {
        let mut d = Detector::new();
        let caps = d
            .feed(b"\x1b]11;rgb:1a1a/1b1b/2626\x1b\\\x1b]10;rgb:a9/b1/d6\x07\x1b]4;4;rgb:7a7a/a2a2/f7f7\x1b\\\x1b[?62c")
            .unwrap();
        assert_eq!(caps.background, Some([0x1a, 0x1b, 0x26]));
        assert_eq!(caps.foreground, Some([0xa9, 0xb1, 0xd6]));
        assert_eq!(caps.palette[4], Some([0x7a, 0xa2, 0xf7]));
        assert!(caps.full_palette().is_none());
    }

    #[test]
    fn timeout_keeps_partial_knowledge() {
        let mut d = Detector::new();
        assert!(d.feed(KITTY_OK).is_none());
        assert!(d.finish_timeout().kitty_graphics);
    }
}
