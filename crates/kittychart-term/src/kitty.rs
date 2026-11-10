//! Encoder for the [kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/).
//!
//! The commands are APC sequences: `ESC _ G <key=value,...> ; <base64 payload> ESC \`.

use std::fmt::Write as _;
use std::io::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

use crate::RgbaImage;

/// The protocol caps each escape's payload at 4096 bytes of base64.
pub const MAX_CHUNK: usize = 4096;

/// How the terminal should acknowledge commands (`q=` key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quiet {
    /// Report success and errors (`q=0`). Useful for diagnostics and tests.
    Verbose,
    /// Suppress OK replies and report errors only (`q=1`).
    ErrorsOnly,
    /// Suppress all replies (`q=2`). This is the default: unread replies would otherwise land in
    /// the host application's stdin.
    #[default]
    Silent,
}

impl Quiet {
    fn value(self) -> u8 {
        match self {
            Quiet::Verbose => 0,
            Quiet::ErrorsOnly => 1,
            Quiet::Silent => 2,
        }
    }
}

/// Options for transmitting an image and displaying it at the cursor (`a=T`).
#[derive(Debug, Clone)]
pub struct TransmitOptions {
    /// Client-chosen image id (`i=`). Retransmitting the same id replaces the image in place,
    /// which is how chart updates avoid flicker. Must be non-zero.
    pub image_id: u32,
    /// Optional placement id (`p=`).
    pub placement_id: Option<u32>,
    /// Scale the image into this many cells (`c=`, `r=`). `None` uses the image's natural size.
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    /// zlib-compress the payload (`o=z`). Charts are mostly flat color and compress very well.
    pub compress: bool,
    /// Leave the cursor where it was instead of moving it past the image (`C=1`).
    pub hold_cursor: bool,
    pub quiet: Quiet,
}

impl TransmitOptions {
    pub fn new(image_id: u32) -> Self {
        assert_ne!(image_id, 0, "kitty image id 0 is reserved");
        Self {
            image_id,
            placement_id: None,
            columns: None,
            rows: None,
            compress: true,
            hold_cursor: false,
            quiet: Quiet::default(),
        }
    }
}

/// Encode an RGBA image as a transmit-and-display command (`a=T,f=32`), chunked per the spec.
///
/// The first chunk carries every control key. Later chunks carry only `m` (more-follows) and `q`.
pub fn transmit_and_display(img: RgbaImage<'_>, opts: &TransmitOptions, out: &mut Vec<u8>) {
    let payload = if opts.compress {
        let mut enc = flate2::write::ZlibEncoder::new(
            Vec::with_capacity(img.data.len() / 8),
            flate2::Compression::fast(),
        );
        enc.write_all(img.data).expect("writing to Vec cannot fail");
        enc.finish().expect("writing to Vec cannot fail")
    } else {
        img.data.to_vec()
    };
    let b64 = B64.encode(&payload);

    let mut keys = String::with_capacity(96);
    write!(keys, "a=T,f=32,s={},v={},i={}", img.width, img.height, opts.image_id).unwrap();
    if let Some(p) = opts.placement_id {
        write!(keys, ",p={p}").unwrap();
    }
    if let Some(c) = opts.columns {
        write!(keys, ",c={c}").unwrap();
    }
    if let Some(r) = opts.rows {
        write!(keys, ",r={r}").unwrap();
    }
    if opts.compress {
        keys.push_str(",o=z");
    }
    if opts.hold_cursor {
        keys.push_str(",C=1");
    }
    write!(keys, ",q={}", opts.quiet.value()).unwrap();

    write_chunked(&keys, opts.quiet, b64.as_bytes(), out);
}

/// Transmit an image and create a *virtual* placement for Unicode placeholders (`a=T,U=1`).
///
/// Nothing is drawn and the cursor doesn't move. The image appears wherever the terminal finds
/// placeholder cells ([`placeholder_cell`]) whose foreground color encodes this image id. This
/// is how images live inside text-based UIs (ratatui, Textual) and tmux: they move, scroll and
/// clip with the text grid. `opts.columns`/`rows` set the placement's size in cells.
pub fn transmit_virtual(img: RgbaImage<'_>, opts: &TransmitOptions, out: &mut Vec<u8>) {
    let mut o = opts.clone();
    o.hold_cursor = false;
    let mut tmp = Vec::new();
    transmit_and_display(img, &o, &mut tmp);
    // Insert U=1 into the first chunk's control data: "\x1b_Ga=T," -> "\x1b_Ga=T,U=1,".
    let marker = b"\x1b_Ga=T,";
    debug_assert!(tmp.starts_with(marker));
    out.extend_from_slice(marker);
    out.extend_from_slice(b"U=1,");
    out.extend_from_slice(&tmp[marker.len()..]);
}

/// The Unicode placeholder character.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// One placeholder cell: U+10EEEE + row and column diacritics, and a third diacritic for the
/// image id's most significant byte when it is non-zero. Its foreground color must encode the
/// low 24 bits of the image id ([`placeholder_fg`]).
pub fn placeholder_cell(row: u32, col: u32, image_id: u32) -> String {
    use crate::diacritics::DIACRITICS;
    let d = |n: u32| char::from_u32(DIACRITICS[n as usize % DIACRITICS.len()]).expect("valid diacritic");
    let mut s = String::with_capacity(12);
    s.push(PLACEHOLDER);
    s.push(d(row));
    s.push(d(col));
    let msb = image_id >> 24;
    if msb != 0 {
        s.push(d(msb));
    }
    s
}

/// The number a row/column diacritic encodes (the inverse of [`placeholder_cell`]'s encoding).
pub fn diacritic_index(c: char) -> Option<u32> {
    crate::diacritics::DIACRITICS.iter().position(|&d| d == c as u32).map(|i| i as u32)
}

/// The 24-bit foreground color that identifies `image_id` in placeholder cells.
pub fn placeholder_fg(image_id: u32) -> (u8, u8, u8) {
    ((image_id >> 16) as u8, (image_id >> 8) as u8, image_id as u8)
}

/// Placeholder text for a `cols × rows` virtual placement: one line per row, each wrapped in
/// the id's foreground color and reset. Rows are separated by `\r\n` and the final row has none.
pub fn placeholder_text(image_id: u32, cols: u32, rows: u32) -> String {
    let (r, g, b) = placeholder_fg(image_id);
    let mut s = String::new();
    for row in 0..rows {
        if row > 0 {
            s.push_str("\r\n");
        }
        s.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
        for col in 0..cols {
            s.push_str(&placeholder_cell(row, col, image_id));
        }
        s.push_str("\x1b[39m");
    }
    s
}

/// Wrap escape sequences for tmux passthrough (`DCS tmux; … ST`, with ESC doubled). tmux must
/// have `set -g allow-passthrough on`. Each escape sequence in `bytes` is wrapped separately,
/// which keeps every passthrough block small.
pub fn tmux_passthrough(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + bytes.len() / 64 + 16);
    let mut i = 0;
    while i < bytes.len() {
        // Find the end of this escape sequence (ST = ESC \\), or pass plain bytes through.
        if bytes[i] == 0x1b {
            let end =
                bytes[i + 1..].windows(2).position(|w| w == b"\x1b\\").map_or(bytes.len(), |p| i + 1 + p + 2);
            out.extend_from_slice(b"\x1bPtmux;");
            for &b in &bytes[i..end] {
                if b == 0x1b {
                    out.push(0x1b);
                }
                out.push(b);
            }
            out.extend_from_slice(b"\x1b\\");
            i = end;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}

/// Delete an image by id together with its placements and stored data (`a=d,d=I`).
pub fn delete_image(image_id: u32, quiet: Quiet, out: &mut Vec<u8>) {
    write!(DisplayVec(out), "\x1b_Ga=d,d=I,i={image_id},q={}\x1b\\", quiet.value()).unwrap();
}

/// A query that asks whether the terminal supports the protocol (`a=q`). It sends a 1×1 RGB
/// pixel, which supporting terminals answer with `ESC _ G i=<id>;OK ESC \` and do not display.
pub fn support_query(image_id: u32, out: &mut Vec<u8>) {
    write!(DisplayVec(out), "\x1b_Gi={image_id},s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\").unwrap();
}

fn write_chunked(keys: &str, quiet: Quiet, b64: &[u8], out: &mut Vec<u8>) {
    // At least one chunk, even for an empty payload.
    let chunks: Vec<&[u8]> = if b64.is_empty() { vec![&[]] } else { b64.chunks(MAX_CHUNK).collect() };
    let last = chunks.len() - 1;
    out.reserve(b64.len() + chunks.len() * 16 + keys.len());
    for (i, chunk) in chunks.into_iter().enumerate() {
        let more = u8::from(i != last);
        out.extend_from_slice(b"\x1b_G");
        if i == 0 {
            out.extend_from_slice(keys.as_bytes());
            if last != 0 {
                write!(DisplayVec(out), ",m={more}").unwrap();
            }
        } else {
            write!(DisplayVec(out), "m={more},q={}", quiet.value()).unwrap();
        }
        out.push(b';');
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// Adapter that lets `write!` target a `Vec<u8>` through `fmt::Write`.
struct DisplayVec<'a>(&'a mut Vec<u8>);
impl std::fmt::Write for DisplayVec<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_chunk_has_no_m_key() {
        let px = [255u8, 0, 0, 255];
        let mut out = Vec::new();
        let mut o = TransmitOptions::new(7);
        o.compress = false;
        transmit_and_display(RgbaImage::new(1, 1, &px), &o, &mut out);
        assert_eq!(out, b"\x1b_Ga=T,f=32,s=1,v=1,i=7,q=2;/wAA/w==\x1b\\");
    }

    #[test]
    fn placeholders_and_tmux() {
        assert_eq!(placeholder_cell(0, 2, 7), "\u{10EEEE}\u{0305}\u{030E}");
        assert_eq!(placeholder_cell(1, 0, 0x0300_0001).chars().count(), 4, "high byte adds a diacritic");
        assert_eq!(placeholder_fg(0x0012_3456), (0x12, 0x34, 0x56));
        assert_eq!(tmux_passthrough(b"\x1b_Ga=q\x1b\\x"), b"\x1bPtmux;\x1b\x1b_Ga=q\x1b\x1b\\\x1b\\x");
    }

    #[test]
    fn delete_command() {
        let mut out = Vec::new();
        delete_image(9, Quiet::Silent, &mut out);
        assert_eq!(out, b"\x1b_Ga=d,d=I,i=9,q=2\x1b\\");
    }
}
