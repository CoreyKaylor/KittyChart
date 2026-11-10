//! Decoder for kitty graphics protocol output.
//!
//! It is written independently from the encoder in `kittychart-term`, from the spec and not from
//! shared code, so round-trip tests catch real encoding mistakes.

use std::collections::BTreeMap;
use std::io::Read as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;

/// One `ESC _ G ... ESC \` command as it appeared in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub keys: BTreeMap<String, String>,
    /// Raw payload text (base64 for transmissions).
    pub payload: String,
}

impl Command {
    pub fn get(&self, k: &str) -> Option<&str> {
        self.keys.get(k).map(String::as_str)
    }
    fn num(&self, k: &str) -> Option<u32> {
        self.get(k).and_then(|v| v.parse().ok())
    }
}

/// Split a byte stream into kitty graphics commands, ignoring everything else.
pub fn parse_commands(bytes: &[u8]) -> Vec<Command> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = find(&bytes[i..], b"\x1b_G") {
        let start = i + p + 3;
        let Some(len) = find(&bytes[start..], b"\x1b\\") else { break };
        let body = std::str::from_utf8(&bytes[start..start + len]).expect("kitty command is ASCII");
        let (ctrl, payload) = body.split_once(';').unwrap_or((body, ""));
        let keys = ctrl
            .split(',')
            .filter(|kv| !kv.is_empty())
            .map(|kv| {
                let (k, v) = kv.split_once('=').unwrap_or_else(|| panic!("malformed key {kv:?}"));
                (k.to_owned(), v.to_owned())
            })
            .collect();
        out.push(Command { keys, payload: payload.to_owned() });
        i = start + len + 2;
    }
    out
}

/// A complete (re-assembled and decoded) image transmission.
#[derive(Debug, Clone)]
pub struct Transmission {
    /// Control keys from the first chunk.
    pub keys: BTreeMap<String, String>,
    pub image_id: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Pixel data as RGBA8. RGB (`f=24`) is expanded with alpha 255.
    pub rgba: Vec<u8>,
    /// Number of escape sequences the transmission was split into.
    pub chunks: usize,
}

/// Re-assemble chunked transmissions (`a=t` / `a=T` / `a=q`) and decode their pixels.
/// Checks the spec's chunking rules along the way: chunk size ≤ 4096, non-final chunks a
/// multiple of 4, and only `m`/`q` keys on continuation chunks.
pub fn decode_transmissions(bytes: &[u8]) -> Result<Vec<Transmission>, String> {
    let cmds = parse_commands(bytes);
    if let Some(c) = cmds.iter().find(|c| c.payload.len() > 4096) {
        return Err(format!("chunk payload {} exceeds 4096", c.payload.len()));
    }
    let mut out = Vec::new();
    let mut it = cmds.into_iter().peekable();
    while let Some(first) = it.next() {
        let action = first.get("a").unwrap_or("t");
        if !matches!(action, "t" | "T" | "q") {
            continue;
        }
        let mut b64 = first.payload.clone();
        let mut chunks = 1;
        let mut more = first.get("m") == Some("1");
        let mut prev_len = first.payload.len();
        while more {
            if prev_len % 4 != 0 {
                return Err(format!("non-final chunk length {prev_len} is not a multiple of 4"));
            }
            let c = it.next().ok_or("stream ended mid-transmission (m=1 with no follow-up)")?;
            if let Some(k) = c.keys.keys().find(|k| !matches!(k.as_str(), "m" | "q")) {
                return Err(format!("continuation chunk carries forbidden key {k:?}"));
            }
            more = c.get("m") == Some("1");
            prev_len = c.payload.len();
            b64.push_str(&c.payload);
            chunks += 1;
        }

        let mut data = B64.decode(&b64).map_err(|e| format!("base64: {e}"))?;
        if first.get("o") == Some("z") {
            let mut raw = Vec::new();
            flate2::read::ZlibDecoder::new(&data[..])
                .read_to_end(&mut raw)
                .map_err(|e| format!("zlib: {e}"))?;
            data = raw;
        }
        let (w, h) = (first.num("s").unwrap_or(0), first.num("v").unwrap_or(0));
        let rgba = match first.num("f").unwrap_or(32) {
            32 => data,
            24 => data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
            f => return Err(format!("unsupported format f={f} in test decoder")),
        };
        if rgba.len() != w as usize * h as usize * 4 {
            return Err(format!("pixel data is {} bytes, expected {w}x{h}x4", rgba.len()));
        }
        out.push(Transmission {
            image_id: first.num("i"),
            keys: first.keys,
            width: w,
            height: h,
            rgba,
            chunks,
        });
    }
    Ok(out)
}

/// Render a byte stream as readable text for snapshot tests. Escapes are shown symbolically,
/// and each payload is replaced by its length, so snapshots stay small and diffs stay meaningful.
pub fn describe(bytes: &[u8]) -> String {
    let mut s = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"\x1b_G") {
            let end = find(&bytes[i..], b"\x1b\\").map_or(bytes.len(), |e| i + e);
            let body = String::from_utf8_lossy(&bytes[i + 3..end]);
            let (ctrl, payload) = body.split_once(';').unwrap_or((&body, ""));
            s.push_str(&format!("APC G {ctrl} ; <{} bytes>\n", payload.len()));
            i = end + 2;
        } else {
            let next = find(&bytes[i + 1..], b"\x1b_G").map_or(bytes.len(), |n| i + 1 + n);
            s.push_str(&format!("{}\n", bytes[i..next].escape_ascii()));
            i = next;
        }
    }
    s
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}
