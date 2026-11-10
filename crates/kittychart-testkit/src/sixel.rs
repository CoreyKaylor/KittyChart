//! An independent sixel decoder, so encoder tests round-trip without a terminal.

/// A decoded sixel image: RGBA with alpha 0 where no sixel bit was set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// The declared raster size (`"Pan;Pad;Ph;Pv`), if present.
    pub raster: Option<(u32, u32)>,
    pub transparent_background: bool,
}

/// Decode the first sixel image (`ESC P … q … ESC \`) in `bytes`.
///
/// # Panics
/// If there is no sixel image, or it is malformed.
pub fn decode(bytes: &[u8]) -> Decoded {
    let start = bytes.windows(2).position(|w| w == b"\x1bP").expect("no DCS");
    let q = start + bytes[start..].iter().position(|&b| b == b'q').expect("no sixel introducer");
    let params: Vec<u32> = std::str::from_utf8(&bytes[start + 2..q])
        .unwrap()
        .split(';')
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let transparent_background = params.get(1) == Some(&1);
    let end = q + bytes[q..].windows(2).position(|w| w == b"\x1b\\").expect("no ST");
    let body = &bytes[q + 1..end];

    let mut palette = vec![[0u8; 3]; 256];
    let mut pixels: std::collections::HashMap<(u32, u32), [u8; 3]> = Default::default();
    let (mut x, mut y, mut color) = (0u32, 0u32, 0usize);
    let mut raster = None;
    let mut i = 0;
    let num = |i: &mut usize| -> u32 {
        let s = *i;
        while *i < body.len() && body[*i].is_ascii_digit() {
            *i += 1;
        }
        std::str::from_utf8(&body[s..*i]).unwrap().parse().unwrap_or(0)
    };
    while i < body.len() {
        let b = body[i];
        i += 1;
        match b {
            b'"' => {
                let mut v = vec![num(&mut i)];
                while i < body.len() && body[i] == b';' {
                    i += 1;
                    v.push(num(&mut i));
                }
                if v.len() == 4 {
                    raster = Some((v[2], v[3]));
                }
            }
            b'#' => {
                let mut v = vec![num(&mut i)];
                while i < body.len() && body[i] == b';' {
                    i += 1;
                    v.push(num(&mut i));
                }
                color = v[0] as usize;
                if v.len() == 5 {
                    assert_eq!(v[1], 2, "only RGB color definitions are expected");
                    palette[color] = [v[2], v[3], v[4]].map(|p| ((p * 255 + 50) / 100) as u8);
                }
            }
            b'!' => {
                let n = num(&mut i);
                let ch = body[i];
                i += 1;
                for _ in 0..n {
                    put(&mut pixels, x, y, ch, palette[color]);
                    x += 1;
                }
            }
            b'$' => x = 0,
            b'-' => {
                x = 0;
                y += 6;
            }
            63..=126 => {
                put(&mut pixels, x, y, b, palette[color]);
                x += 1;
            }
            _ => {}
        }
    }
    let (width, height) = raster.unwrap_or_else(|| {
        let w = pixels.keys().map(|k| k.0 + 1).max().unwrap_or(0);
        let h = pixels.keys().map(|k| k.1 + 1).max().unwrap_or(0);
        (w, h)
    });
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for ((px, py), c) in pixels {
        if px < width && py < height {
            let o = ((py * width + px) * 4) as usize;
            rgba[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    Decoded { width, height, rgba, raster, transparent_background }
}

fn put(pixels: &mut std::collections::HashMap<(u32, u32), [u8; 3]>, x: u32, y: u32, ch: u8, c: [u8; 3]) {
    let bits = ch - 63;
    for dy in 0..6 {
        if bits & (1 << dy) != 0 {
            pixels.insert((x, y + dy), c);
        }
    }
}
