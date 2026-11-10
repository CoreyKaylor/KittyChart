//! Sans-IO terminal input: bytes in, [`Event`]s out.
//!
//! Handles what an interactive chart needs: keys (including arrows and other CSI/SS3 keys),
//! SGR mouse reports in cell (mode 1006) or pixel (mode 1016) coordinates, focus in/out
//! (1004), bracketed paste (2004) and in-band window size reports (2048, `CSI 48 ; …t`).
//! Escape sequences split across reads are buffered until complete; a lone ESC is reported as
//! the Escape key once [`Parser::flush`] is called (after a short idle timeout).

/// Mode switches for interactive use. [`ENABLE`] turns them on, [`DISABLE`] off (in reverse).
///
/// - 1049: alternate screen, 25 (l): hide the cursor
/// - 1003: report all mouse motion; 1006: SGR encoding; 1016: SGR pixel coordinates
/// - 1004: focus events; 2004: bracketed paste; 2048: in-band resize reports
pub const ENABLE: &str =
    "\x1b[?1049h\x1b[?25l\x1b[?1003h\x1b[?1006h\x1b[?1016h\x1b[?1004h\x1b[?2004h\x1b[?2048h";
/// Undo [`ENABLE`].
pub const DISABLE: &str =
    "\x1b[?2048l\x1b[?2004l\x1b[?1004l\x1b[?1016l\x1b[?1006l\x1b[?1003l\x1b[?25h\x1b[?1049l";

/// A key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Delete,
    Insert,
    F(u8),
}

/// Modifier keys held with a key or mouse event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    /// From an xterm modifier parameter (`1 + bits`).
    fn from_param(p: u32) -> Mods {
        let b = p.saturating_sub(1);
        Mods { shift: b & 1 != 0, alt: b & 2 != 0, ctrl: b & 4 != 0 }
    }
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    /// Motion with no button held.
    None,
}

/// What happened with the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Release,
    Move,
}

/// A mouse event. Coordinates are 0-based: cells for mode 1006, pixels for mode 1016 (the
/// terminal decides; [`Mouse::pixels`] says which this parser was told to expect).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mouse {
    pub kind: MouseKind,
    pub button: Button,
    pub x: u32,
    pub y: u32,
    pub mods: Mods,
    pub pixels: bool,
}

/// An input event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Key(Key, Mods),
    Mouse(Mouse),
    FocusIn,
    FocusOut,
    Paste(String),
    /// In-band resize (mode 2048): cells and pixels.
    Resize {
        cols: u16,
        rows: u16,
        px_width: u16,
        px_height: u16,
    },
}

/// Input parser. Feed it bytes as they arrive.
#[derive(Debug, Default)]
pub struct Parser {
    buf: Vec<u8>,
    paste: Option<Vec<u8>>,
    /// Mouse coordinates are pixels (mode 1016 is on).
    pub pixel_mouse: bool,
}

impl Parser {
    pub fn new(pixel_mouse: bool) -> Parser {
        Parser { pixel_mouse, ..Parser::default() }
    }

    /// Parse `bytes`, returning complete events. Incomplete sequences wait for more input.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Event> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            if let Some(paste) = &mut self.paste {
                // Inside bracketed paste: everything up to ESC [ 201 ~ is text.
                const END: &[u8] = b"\x1b[201~";
                match find(&self.buf, END) {
                    Some(i) => {
                        paste.extend_from_slice(&self.buf[..i]);
                        let text = String::from_utf8_lossy(paste).into_owned();
                        self.paste = None;
                        self.buf.drain(..i + END.len());
                        out.push(Event::Paste(text));
                        continue;
                    }
                    None => {
                        // Keep a possible partial terminator.
                        let keep = (1..END.len()).rev().find(|&k| self.buf.ends_with(&END[..k])).unwrap_or(0);
                        let take = self.buf.len() - keep;
                        paste.extend_from_slice(&self.buf[..take]);
                        self.buf.drain(..take);
                        return out;
                    }
                }
            }
            if self.buf.is_empty() {
                return out;
            }
            match self.parse_one() {
                Step::Event(e, n) => {
                    self.buf.drain(..n);
                    if let Some(e) = e {
                        out.push(e);
                    }
                }
                Step::PasteStart(n) => {
                    self.buf.drain(..n);
                    self.paste = Some(Vec::new());
                }
                Step::Incomplete => return out,
            }
        }
    }

    /// Report a lone, buffered ESC as the Escape key (call after an idle timeout: a real
    /// sequence's bytes arrive together).
    pub fn flush(&mut self) -> Vec<Event> {
        if self.paste.is_none() && self.buf == b"\x1b" {
            self.buf.clear();
            return vec![Event::Key(Key::Escape, Mods::default())];
        }
        vec![]
    }

    fn parse_one(&self) -> Step {
        let b = &self.buf;
        match b[0] {
            0x1b => self.parse_escape(),
            b'\r' | b'\n' => Step::Event(Some(Event::Key(Key::Enter, Mods::default())), 1),
            b'\t' => Step::Event(Some(Event::Key(Key::Tab, Mods::default())), 1),
            0x7f | 0x08 => Step::Event(Some(Event::Key(Key::Backspace, Mods::default())), 1),
            c @ 0x01..=0x1a => {
                let ch = (b'a' + c - 1) as char;
                Step::Event(Some(Event::Key(Key::Char(ch), Mods { ctrl: true, ..Mods::default() })), 1)
            }
            _ => {
                // UTF-8 character.
                let len = match b[0] {
                    0x00..=0x7f => 1,
                    0xc0..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf7 => 4,
                    _ => return Step::Event(None, 1),
                };
                if b.len() < len {
                    return Step::Incomplete;
                }
                match std::str::from_utf8(&b[..len]).ok().and_then(|s| s.chars().next()) {
                    Some(ch) => Step::Event(Some(Event::Key(Key::Char(ch), Mods::default())), len),
                    None => Step::Event(None, 1),
                }
            }
        }
    }

    fn parse_escape(&self) -> Step {
        let b = &self.buf;
        if b.len() < 2 {
            return Step::Incomplete;
        }
        match b[1] {
            b'[' => self.parse_csi(),
            b'O' => {
                // SS3: F1–F4 and application-mode arrows.
                if b.len() < 3 {
                    return Step::Incomplete;
                }
                let key = match b[2] {
                    b'A' => Some(Key::Up),
                    b'B' => Some(Key::Down),
                    b'C' => Some(Key::Right),
                    b'D' => Some(Key::Left),
                    b'H' => Some(Key::Home),
                    b'F' => Some(Key::End),
                    b'P' => Some(Key::F(1)),
                    b'Q' => Some(Key::F(2)),
                    b'R' => Some(Key::F(3)),
                    b'S' => Some(Key::F(4)),
                    _ => None,
                };
                Step::Event(key.map(|k| Event::Key(k, Mods::default())), 3)
            }
            0x1b => Step::Event(Some(Event::Key(Key::Escape, Mods::default())), 1),
            c => {
                // Alt + key.
                let inner = Parser { buf: b[1..].to_vec(), paste: None, pixel_mouse: self.pixel_mouse };
                match inner.parse_one() {
                    Step::Event(Some(Event::Key(k, mut m)), n) => {
                        m.alt = true;
                        Step::Event(Some(Event::Key(k, m)), n + 1)
                    }
                    Step::Incomplete => Step::Incomplete,
                    _ => {
                        let _ = c;
                        Step::Event(None, 2)
                    }
                }
            }
        }
    }

    fn parse_csi(&self) -> Step {
        let b = &self.buf;
        // ESC [ params intermediates final
        let mut i = 2;
        let private = b.get(i).copied().filter(|c| matches!(c, b'<' | b'?' | b'>' | b'='));
        if private.is_some() {
            i += 1;
        }
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b';' || b[i] == b':') {
            i += 1;
        }
        if i >= b.len() {
            return Step::Incomplete;
        }
        let params: Vec<u32> = std::str::from_utf8(&b[start..i])
            .unwrap_or("")
            .split([';', ':'])
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        let fin = b[i];
        let n = i + 1;
        let p = |k: usize| params.get(k).copied().unwrap_or(0);
        let mods = Mods::from_param(p(1).max(1));
        let key = |k: Key| Step::Event(Some(Event::Key(k, mods)), n);
        match (private, fin) {
            (Some(b'<'), b'M' | b'm') => {
                let code = p(0);
                let (x, y) = (p(1).saturating_sub(1), p(2).saturating_sub(1));
                let motion = code & 32 != 0;
                let base = code & !(4 | 8 | 16 | 32);
                let button = match base {
                    0 => Button::Left,
                    1 => Button::Middle,
                    2 => Button::Right,
                    3 => Button::None,
                    64 => Button::WheelUp,
                    65 => Button::WheelDown,
                    66 => Button::WheelLeft,
                    67 => Button::WheelRight,
                    _ => Button::None,
                };
                let kind = if fin == b'm' {
                    MouseKind::Release
                } else if motion {
                    MouseKind::Move
                } else {
                    MouseKind::Press
                };
                let mods = Mods { shift: code & 4 != 0, alt: code & 8 != 0, ctrl: code & 16 != 0 };
                Step::Event(
                    Some(Event::Mouse(Mouse { kind, button, x, y, mods, pixels: self.pixel_mouse })),
                    n,
                )
            }
            (None, b'A') => key(Key::Up),
            (None, b'B') => key(Key::Down),
            (None, b'C') => key(Key::Right),
            (None, b'D') => key(Key::Left),
            (None, b'H') => key(Key::Home),
            (None, b'F') => key(Key::End),
            (None, b'Z') => {
                Step::Event(Some(Event::Key(Key::Tab, Mods { shift: true, ..Mods::default() })), n)
            }
            (None, b'I') => Step::Event(Some(Event::FocusIn), n),
            (None, b'O') => Step::Event(Some(Event::FocusOut), n),
            (None, b'~') => match p(0) {
                200 => Step::PasteStart(n),
                1 | 7 => key(Key::Home),
                2 => key(Key::Insert),
                3 => key(Key::Delete),
                4 | 8 => key(Key::End),
                5 => key(Key::PageUp),
                6 => key(Key::PageDown),
                k @ 11..=15 => key(Key::F((k - 10) as u8)),
                k @ 17..=21 => key(Key::F((k - 11) as u8)),
                k @ 23..=24 => key(Key::F((k - 12) as u8)),
                _ => Step::Event(None, n),
            },
            (None, b't') if p(0) == 48 => Step::Event(
                Some(Event::Resize {
                    rows: p(1) as u16,
                    cols: p(2) as u16,
                    px_height: p(3) as u16,
                    px_width: p(4) as u16,
                }),
                n,
            ),
            // kitty keyboard protocol and others we don't use: consumed.
            _ => Step::Event(None, n),
        }
    }
}

enum Step {
    Event(Option<Event>, usize),
    PasteStart(usize),
    Incomplete,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(bytes: &[u8]) -> Vec<Event> {
        Parser::new(true).feed(bytes)
    }

    #[test]
    fn keys_and_modifiers() {
        let none = Mods::default();
        assert_eq!(keys(b"q"), [Event::Key(Key::Char('q'), none)]);
        assert_eq!(keys("é".as_bytes()), [Event::Key(Key::Char('é'), none)]);
        assert_eq!(
            keys(b"\x1b[A\x1bOB\r"),
            [Event::Key(Key::Up, none), Event::Key(Key::Down, none), Event::Key(Key::Enter, none)]
        );
        assert_eq!(keys(b"\x1b[1;5C"), [Event::Key(Key::Right, Mods { ctrl: true, ..none })]);
        assert_eq!(keys(b"\x03"), [Event::Key(Key::Char('c'), Mods { ctrl: true, ..none })]);
        assert_eq!(keys(b"\x1bx"), [Event::Key(Key::Char('x'), Mods { alt: true, ..none })]);
        assert_eq!(keys(b"\x1b[5~\x1b[15~"), [Event::Key(Key::PageUp, none), Event::Key(Key::F(5), none)]);
    }

    #[test]
    fn sgr_mouse_in_pixels() {
        let ev = keys(b"\x1b[<35;120;48M\x1b[<0;10;20M\x1b[<0;10;20m\x1b[<65;1;1M");
        let m = |kind, button, x, y| {
            Event::Mouse(Mouse { kind, button, x, y, mods: Mods::default(), pixels: true })
        };
        assert_eq!(
            ev,
            [
                m(MouseKind::Move, Button::None, 119, 47),
                m(MouseKind::Press, Button::Left, 9, 19),
                m(MouseKind::Release, Button::Left, 9, 19),
                m(MouseKind::Press, Button::WheelDown, 0, 0),
            ]
        );
    }

    #[test]
    fn split_sequences_paste_focus_resize() {
        let mut p = Parser::new(false);
        assert!(p.feed(b"\x1b[<0;3").is_empty(), "incomplete");
        assert_eq!(p.feed(b";4M").len(), 1);
        assert!(p.feed(b"\x1b").is_empty());
        assert_eq!(p.flush(), [Event::Key(Key::Escape, Mods::default())]);
        assert_eq!(p.feed(b"\x1b[200~hi \x1b[A there\x1b[20"), []);
        assert_eq!(
            p.feed(b"1~x"),
            [Event::Paste("hi \x1b[A there".into()), Event::Key(Key::Char('x'), Mods::default())]
        );
        assert_eq!(p.feed(b"\x1b[I\x1b[O"), [Event::FocusIn, Event::FocusOut]);
        assert_eq!(
            p.feed(b"\x1b[48;24;80;480;800t"),
            [Event::Resize { rows: 24, cols: 80, px_height: 480, px_width: 800 }]
        );
    }
}
