//! Dev-only support for checking KittyChart against real terminal implementations.
//!
//! - [`answering_terminal`]: Ghostty's VT core, set up to answer the queries a program sends
//!   (DA1, cell size, kitty graphics, OSC colors), with chosen colors.
//! - [`pty`]: run a real program on a pseudo-terminal attached to that emulator.
//! - [`screen_text`]: what the emulator's screen shows, as text.
//!
//! Used by this crate's tests and by `kc-preview`.

pub mod pty;

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::render::{CellIterator, RowIterator};
use libghostty_vt::style::{Palette, RgbColor};
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, PrimaryDeviceAttributes,
    SecondaryDeviceAttributes, SizeReportSize, TertiaryDeviceAttributes,
};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

/// Terminal colors: background, foreground, cursor, ANSI 0–15.
#[derive(Debug, Clone)]
pub struct Colors {
    pub background: [u8; 3],
    pub foreground: [u8; 3],
    pub cursor: [u8; 3],
    pub palette: [[u8; 3]; 16],
}

impl Default for Colors {
    /// Tokyo Night.
    fn default() -> Self {
        let hex = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        Colors {
            background: hex(0x1a1b26),
            foreground: hex(0xa9b1d6),
            cursor: hex(0xc0caf5),
            palette: [
                0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, 0x414868,
                0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
            ]
            .map(hex),
        }
    }
}

fn rgb(c: [u8; 3]) -> RgbColor {
    RgbColor { r: c[0], g: c[1], b: c[2] }
}

/// A `cols × rows` terminal of `cw × ch`-pixel cells that answers queries; its replies collect
/// in the returned buffer (write them back to the program).
pub fn answering_terminal(
    cols: u16,
    rows: u16,
    cw: u32,
    ch: u32,
    colors: &Colors,
) -> (Terminal<'static, 'static>, Rc<RefCell<Vec<u8>>>) {
    let mut t = Terminal::new(TerminalOptions { cols, rows, max_scrollback: 0 }).expect("terminal");
    t.resize(cols, rows, cw, ch).expect("resize");
    t.set_kitty_image_storage_limit(256 * 1024 * 1024).expect("kitty graphics");
    t.set_default_bg_color(Some(rgb(colors.background))).expect("bg");
    t.set_default_fg_color(Some(rgb(colors.foreground))).expect("fg");
    t.set_default_cursor_color(Some(rgb(colors.cursor))).expect("cursor");
    let mut pal = t.default_color_palette().expect("palette").0;
    for (i, c) in colors.palette.iter().enumerate() {
        pal[i] = rgb(*c);
    }
    t.set_default_color_palette(Some(Palette(pal))).expect("palette");

    let replies = Rc::new(RefCell::new(Vec::new()));
    let r = replies.clone();
    t.on_pty_write(move |_, d| r.borrow_mut().extend_from_slice(d)).expect("pty");
    t.on_device_attributes(|_| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[DeviceAttributeFeature::ANSI_COLOR],
            ),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT220,
                firmware_version: 1,
                rom_cartridge: 0,
            },
            tertiary: TertiaryDeviceAttributes { unit_id: 0 },
        })
    })
    .expect("da");
    t.on_size(move |_| Some(SizeReportSize { rows, columns: cols, cell_width: cw, cell_height: ch }))
        .expect("size");
    (t, replies)
}

/// The screen as text, one line per row (trailing spaces trimmed). Unicode placeholder cells
/// read as their base character, U+10EEEE.
pub fn screen_text(t: &Terminal<'_, '_>) -> String {
    let mut rs = RenderState::new().expect("render state");
    let snap = rs.update(t).expect("snapshot");
    let mut rows_owner = RowIterator::new().expect("rows");
    let mut cells_owner = CellIterator::new().expect("cells");
    let mut rows = rows_owner.update(&snap).expect("row iter");
    let mut lines = Vec::new();
    while let Some(row) = rows.next() {
        let mut cells = cells_owner.update(row).expect("cell iter");
        let mut line = String::new();
        while let Some(cell) = cells.next() {
            let g = cell.graphemes().unwrap_or_default();
            line.push(g.first().copied().unwrap_or(' '));
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}
