//! L3 conformance: feed KittyChart's output into Ghostty's real VT core (libghostty-vt),
//! in-process and headless, then assert on the terminal's resulting state.
//!
//! This catches what our own decoder can't: escapes that are spec-valid by our reading but
//! handled differently by a real terminal (cursor movement, id replacement, chunk handling).
#![allow(clippy::unwrap_used)]

use std::cell::RefCell;
use std::rc::Rc;

use kittychart_term::RgbaImage;
use kittychart_term::detect::Detector;
use kittychart_term::kitty::{self, Quiet, TransmitOptions};
use libghostty_vt::kitty::graphics::{ImageFormat, PlacementIterator};
use libghostty_vt::terminal::{
    ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType, PrimaryDeviceAttributes,
    SecondaryDeviceAttributes, SizeReportSize, TertiaryDeviceAttributes,
};
use libghostty_vt::{Terminal, TerminalOptions};

const COLS: u16 = 80;
const ROWS: u16 = 24;
const CELL_W: u32 = 10;
const CELL_H: u32 = 20;

fn terminal() -> Terminal<'static, 'static> {
    let mut t = Terminal::new(TerminalOptions { cols: COLS, rows: ROWS, max_scrollback: 100 }).unwrap();
    t.resize(COLS, ROWS, CELL_W, CELL_H).unwrap();
    t.set_kitty_image_storage_limit(64 * 1024 * 1024).unwrap();
    t
}

/// Deterministic, poorly compressible test pattern, so payloads span many chunks.
fn pattern(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut x = seed.wrapping_mul(2_654_435_761) | 1;
    (0..w * h * 4)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            if i % 4 == 3 { 255 } else { (x & 0xff) as u8 }
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct Placed {
    image_id: u32,
    col: i32,
    row: i32,
    cols: u32,
    rows: u32,
}

fn placements(t: &Terminal<'_, '_>) -> Vec<Placed> {
    let g = t.kitty_graphics().unwrap();
    let mut iter = PlacementIterator::new().unwrap();
    let mut it = iter.update(&g).unwrap();
    let mut out = Vec::new();
    while let Some(p) = it.next() {
        let img = g.image(p.image_id().unwrap()).unwrap();
        let pos = p.viewport_pos(&img, t).unwrap().expect("placement visible");
        let grid = p.grid_size(&img, t).unwrap();
        out.push(Placed {
            image_id: p.image_id().unwrap(),
            col: pos.col,
            row: pos.row,
            cols: grid.cols,
            rows: grid.rows,
        });
    }
    out
}

fn stored(t: &Terminal<'_, '_>, id: u32) -> Option<(u32, u32, ImageFormat, Vec<u8>)> {
    let g = t.kitty_graphics().unwrap();
    let img = g.image(id)?;
    Some((img.width().unwrap(), img.height().unwrap(), img.format().unwrap(), img.data().unwrap().to_vec()))
}

fn send(t: &mut Terminal<'_, '_>, w: u32, h: u32, px: &[u8], opts: &TransmitOptions) {
    let mut out = Vec::new();
    kitty::transmit_and_display(RgbaImage::new(w, h, px), opts, &mut out);
    t.vt_write(&out);
}

#[test]
fn compressed_rgba_is_stored_intact_and_placed_at_cursor() {
    let mut t = terminal();
    t.vt_write(b"hello\r\n  ");
    let px = pattern(40, 40, 1);
    send(&mut t, 40, 40, &px, &TransmitOptions::new(1));

    let (w, h, fmt, data) = stored(&t, 1).expect("image stored");
    assert_eq!((w, h, fmt), (40, 40, ImageFormat::Rgba));
    assert_eq!(data, px, "pixels survive zlib + base64 + chunking");
    assert_eq!(placements(&t), [Placed { image_id: 1, col: 2, row: 1, cols: 4, rows: 2 }]);
    // Spec: the cursor ends after the image's last column, on its last row.
    assert_eq!((t.cursor_x().unwrap(), t.cursor_y().unwrap()), (6, 2));
}

#[test]
fn large_multi_chunk_transmission() {
    let mut t = terminal();
    let (w, h) = (320, 240); // ~400 KB of noise, about 100 chunks
    let px = pattern(w, h, 2);
    let mut o = TransmitOptions::new(2);
    o.compress = false;
    send(&mut t, w, h, &px, &o);
    assert_eq!(stored(&t, 2).unwrap().3, px);
}

#[test]
fn retransmitting_same_id_replaces_image() {
    let mut t = terminal();
    let o = TransmitOptions::new(3);
    send(&mut t, 20, 20, &pattern(20, 20, 3), &o);
    let second = pattern(30, 10, 4);
    send(&mut t, 30, 10, &second, &o);
    let (w, h, _, data) = stored(&t, 3).unwrap();
    assert_eq!((w, h), (30, 10));
    assert_eq!(data, second);
}

#[test]
fn explicit_cell_size_and_hold_cursor() {
    let mut t = terminal();
    let mut o = TransmitOptions::new(4);
    o.columns = Some(12);
    o.rows = Some(5);
    o.hold_cursor = true;
    send(&mut t, 64, 32, &pattern(64, 32, 5), &o);
    assert_eq!(placements(&t), [Placed { image_id: 4, col: 0, row: 0, cols: 12, rows: 5 }]);
    assert_eq!((t.cursor_x().unwrap(), t.cursor_y().unwrap()), (0, 0));
}

#[test]
fn delete_removes_image_and_placements() {
    let mut t = terminal();
    send(&mut t, 8, 8, &pattern(8, 8, 6), &TransmitOptions::new(5));
    let mut out = Vec::new();
    kitty::delete_image(5, Quiet::Silent, &mut out);
    t.vt_write(&out);
    assert!(stored(&t, 5).is_none());
    assert!(placements(&t).is_empty());
}

#[test]
fn quiet_modes_control_replies() {
    for (quiet, expect_reply) in [(Quiet::Silent, false), (Quiet::Verbose, true)] {
        let mut t = terminal();
        let replies = Rc::new(RefCell::new(Vec::new()));
        let r = replies.clone();
        t.on_pty_write(move |_, data| r.borrow_mut().extend_from_slice(data)).unwrap();
        let mut o = TransmitOptions::new(6);
        o.quiet = quiet;
        send(&mut t, 4, 4, &pattern(4, 4, 7), &o);
        assert_eq!(
            !replies.borrow().is_empty(),
            expect_reply,
            "{quiet:?}: {:?}",
            replies.borrow().escape_ascii().to_string()
        );
    }
}

/// Our sans-IO Detector talking to a real terminal: Ghostty generates the kitty `a=q` reply
/// itself. DA1 and the size report come from embedder callbacks, as they would in a real
/// Ghostty app.
#[test]
fn detector_against_ghostty() {
    let mut t = terminal();
    let replies = Rc::new(RefCell::new(Vec::new()));
    let r = replies.clone();
    t.on_pty_write(move |_, data| r.borrow_mut().extend_from_slice(data)).unwrap();
    t.on_device_attributes(|_| {
        Some(DeviceAttributes {
            primary: PrimaryDeviceAttributes::new(
                ConformanceLevel::VT220,
                &[DeviceAttributeFeature::ANSI_COLOR, DeviceAttributeFeature::CLIPBOARD],
            ),
            secondary: SecondaryDeviceAttributes {
                device_type: DeviceType::VT220,
                firmware_version: 1,
                rom_cartridge: 0,
            },
            tertiary: TertiaryDeviceAttributes { unit_id: 0 },
        })
    })
    .unwrap();
    t.on_size(|_| {
        Some(SizeReportSize { rows: ROWS, columns: COLS, cell_width: CELL_W, cell_height: CELL_H })
    })
    .unwrap();

    t.set_default_bg_color(Some(libghostty_vt::style::RgbColor { r: 0x1a, g: 0x1b, b: 0x26 })).unwrap();
    t.set_default_fg_color(Some(libghostty_vt::style::RgbColor { r: 0xa9, g: 0xb1, b: 0xd6 })).unwrap();

    let mut d = Detector::new();
    t.vt_write(&d.queries());
    let reply = replies.borrow().clone();
    let caps =
        d.feed(&reply).unwrap_or_else(|| panic!("no DA1 sentinel in {:?}", reply.escape_ascii().to_string()));
    assert!(caps.kitty_graphics, "ghostty reply: {:?}", reply.escape_ascii().to_string());
    assert!(!caps.sixel);
    assert_eq!(caps.cell_px, Some((CELL_W as u16, CELL_H as u16)));
    assert!(placements(&t).is_empty(), "a=q must not display anything");
    // Theme colors come back over OSC 10/11/4 in Ghostty's own reply format.
    assert_eq!(caps.background, Some([0x1a, 0x1b, 0x26]));
    assert_eq!(caps.foreground, Some([0xa9, 0xb1, 0xd6]));
    assert!(caps.full_palette().is_some(), "all 16 palette entries reported: {:?}", caps.palette);
}

/// Unicode placeholders: a virtual placement plus placeholder cells whose foreground color
/// carries the image id. This is the path for ratatui/Textual and tmux.
#[test]
fn unicode_placeholders_create_virtual_placement() {
    use libghostty_vt::RenderState;
    use libghostty_vt::render::{CellIterator, RowIterator};

    let mut t = terminal();
    let id = 0x00AB_CDEF;
    let mut o = TransmitOptions::new(id);
    o.columns = Some(6);
    o.rows = Some(2);
    let mut out = Vec::new();
    kitty::transmit_virtual(RgbaImage::new(60, 40, &pattern(60, 40, 9)), &o, &mut out);
    t.vt_write(&out);
    // The transmit must not move the cursor or draw a visible placement.
    assert_eq!((t.cursor_x().unwrap(), t.cursor_y().unwrap()), (0, 0));
    t.vt_write(kitty::placeholder_text(id, 6, 2).as_bytes());

    {
        let g = t.kitty_graphics().unwrap();
        let mut iter = PlacementIterator::new().unwrap();
        let mut ps = iter.update(&g).unwrap();
        let p = ps.next().expect("a placement");
        assert_eq!(p.image_id().unwrap(), id);
        assert!(p.is_virtual().unwrap(), "U=1 creates a virtual placement");
    }

    // Every cell in the 6×2 box holds U+10EEEE in the id's color.
    let mut rs = RenderState::new().unwrap();
    let snap = rs.update(&t).unwrap();
    let mut rows = RowIterator::new().unwrap();
    let mut cells = CellIterator::new().unwrap();
    let mut row_it = rows.update(&snap).unwrap();
    let mut found = 0;
    let mut r = 0;
    while let Some(row) = row_it.next() {
        let mut cell_it = cells.update(row).unwrap();
        let mut c = 0;
        while let Some(cell) = cell_it.next() {
            let gr = cell.graphemes().unwrap();
            if r < 2 && c < 6 {
                assert_eq!(gr.first(), Some(&kitty::PLACEHOLDER), "cell {r},{c}");
                let fg = cell.fg_color().unwrap().expect("explicit fg");
                assert_eq!((fg.r, fg.g, fg.b), (0xAB, 0xCD, 0xEF));
                found += 1;
            }
            c += 1;
        }
        r += 1;
    }
    assert_eq!(found, 12);
}
