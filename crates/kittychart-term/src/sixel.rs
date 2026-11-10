//! Sixel encoder: for terminals with sixel but no kitty graphics (foot, WezTerm, xterm, mlterm,
//! Windows Terminal, iTerm2 …).
//!
//! The image is quantized to at most 256 colors by median cut (charts are mostly flat colors,
//! so the palette is usually exact apart from anti-aliased edges) with no dithering, which would
//! only add noise to flat fills. Transparent pixels are left unset (`P2=1`), so the terminal
//! background shows through. When the terminal's background color is known, translucent
//! pixels are blended onto it first, which keeps anti-aliased edges smooth.

use std::collections::HashMap;

use crate::RgbaImage;

type Rgb = [u8; 3];

/// Encoding options.
#[derive(Debug, Clone, Copy)]
pub struct SixelOptions {
    /// Blend translucent pixels onto this color (the terminal background). Without it, pixels
    /// under half opacity are transparent and the rest opaque.
    pub background: Option<Rgb>,
    /// Palette size, 2..=256. Most terminals support 256 color registers.
    pub max_colors: usize,
    /// Paint every pixel, fully transparent ones in `background` (black without one), and
    /// declare the image opaque (`P2=0`). For images redrawn in place, such as animation
    /// frames: with transparent pixels, terminals keep what was there before (foot keeps the
    /// older images underneath), so the previous frames show through.
    pub opaque: bool,
}

impl Default for SixelOptions {
    fn default() -> Self {
        Self { background: None, max_colors: 256, opaque: false }
    }
}

/// Append a sixel image (`DCS … q … ST`) to `out`. The image is drawn at the cursor.
pub fn encode(img: RgbaImage<'_>, opts: &SixelOptions, out: &mut Vec<u8>) {
    let (w, h) = (img.width as usize, img.height as usize);
    // Flatten to opaque colors or transparency.
    let px: Vec<Option<Rgb>> = img
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| {
            let a = p[3];
            match opts.background {
                _ if a == 0 && opts.opaque => Some(opts.background.unwrap_or([0, 0, 0])),
                _ if a == 0 => None,
                Some(bg) => Some(std::array::from_fn(|i| {
                    ((u32::from(p[i]) * u32::from(a) + u32::from(bg[i]) * (255 - u32::from(a)) + 127) / 255)
                        as u8
                })),
                None if a < 128 && !opts.opaque => None,
                None => Some([p[0], p[1], p[2]]),
            }
        })
        .collect();

    let mut hist: HashMap<Rgb, u32> = HashMap::new();
    for c in px.iter().flatten() {
        *hist.entry(*c).or_default() += 1;
    }
    let palette = median_cut(&hist, opts.max_colors.clamp(2, 256));
    let index: HashMap<Rgb, u8> = hist.keys().map(|&c| (c, nearest(&palette, c))).collect();

    // DCS P1;P2;P3 q. P2 = 1: pixels with no sixel bit set keep the background (P2 = 0 for an
    // opaque image: every pixel is set). The raster attributes ("Pan;Pad;Ph;Pv) set a 1:1 pixel
    // aspect and the image size.
    out.extend_from_slice(if opts.opaque { b"\x1bP0;0;0q" } else { b"\x1bP0;1;0q" });
    out.extend_from_slice(format!("\"1;1;{w};{h}").as_bytes());
    for (i, c) in palette.iter().enumerate() {
        let pct = |v: u8| (u32::from(v) * 100 + 127) / 255;
        out.extend_from_slice(format!("#{i};2;{};{};{}", pct(c[0]), pct(c[1]), pct(c[2])).as_bytes());
    }

    let mut rows: Vec<Option<Vec<u8>>> = vec![None; palette.len()];
    let mut used: Vec<usize> = Vec::new();
    for y0 in (0..h).step_by(6) {
        for dy in 0..6.min(h - y0) {
            for x in 0..w {
                let Some(c) = px[(y0 + dy) * w + x] else { continue };
                let i = usize::from(index[&c]);
                let row = rows[i].get_or_insert_with(|| {
                    used.push(i);
                    vec![0u8; w]
                });
                row[x] |= 1 << dy;
            }
        }
        used.sort_unstable();
        for (n, &i) in used.iter().enumerate() {
            if n > 0 {
                out.push(b'$'); // back to the start of this band for the next color
            }
            out.extend_from_slice(format!("#{i}").as_bytes());
            let row = rows[i].take().unwrap_or_default();
            let end = row.iter().rposition(|&b| b != 0).map_or(0, |p| p + 1);
            rle(&row[..end], out);
        }
        used.clear();
        if y0 + 6 < h {
            out.push(b'-'); // next band
        }
    }
    out.extend_from_slice(b"\x1b\\");
}

/// Sixel characters for one color's row of a band, run-length encoded (`!n c`).
fn rle(bits: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < bits.len() {
        let b = bits[i];
        let mut n = 1;
        while i + n < bits.len() && bits[i + n] == b {
            n += 1;
        }
        let ch = 63 + b;
        if n >= 4 {
            out.extend_from_slice(format!("!{n}").as_bytes());
            out.push(ch);
        } else {
            out.extend(std::iter::repeat_n(ch, n));
        }
        i += n;
    }
}

/// Median cut over the color histogram: split the box with the widest channel range at its
/// pixel-weighted median until there are `max` boxes; each box's weighted mean is a palette
/// color. Exact when there are no more than `max` distinct colors.
fn median_cut(hist: &HashMap<Rgb, u32>, max: usize) -> Vec<Rgb> {
    let mut colors: Vec<(Rgb, u32)> = hist.iter().map(|(&c, &n)| (c, n)).collect();
    colors.sort_unstable(); // deterministic regardless of hash order
    if colors.len() <= max {
        return colors.into_iter().map(|(c, _)| c).collect();
    }
    let range = |b: &[(Rgb, u32)]| -> (usize, u8) {
        (0..3)
            .map(|ch| {
                let (lo, hi) = b.iter().fold((255u8, 0u8), |(lo, hi), (c, _)| (lo.min(c[ch]), hi.max(c[ch])));
                (ch, hi - lo)
            })
            .max_by_key(|&(ch, r)| (r, std::cmp::Reverse(ch)))
            .unwrap_or((0, 0))
    };
    let mut boxes: Vec<Vec<(Rgb, u32)>> = vec![colors];
    while boxes.len() < max {
        // Widest box that can still be split.
        let Some((bi, (ch, _))) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(i, b)| (i, range(b)))
            .max_by_key(|&(i, (_, r))| (r, std::cmp::Reverse(i)))
        else {
            break;
        };
        let mut b = boxes.swap_remove(bi);
        b.sort_unstable_by_key(|(c, _)| (c[ch], *c));
        let total: u64 = b.iter().map(|(_, n)| u64::from(*n)).sum();
        let mut acc = 0u64;
        let mut cut = 1;
        for (k, (_, n)) in b.iter().enumerate() {
            acc += u64::from(*n);
            if acc * 2 >= total {
                cut = (k + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let rest = b.split_off(cut);
        boxes.push(b);
        boxes.push(rest);
    }
    boxes
        .iter()
        .map(|b| {
            let total: u64 = b.iter().map(|(_, n)| u64::from(*n)).sum::<u64>().max(1);
            std::array::from_fn(|ch| {
                (b.iter().map(|(c, n)| u64::from(c[ch]) * u64::from(*n)).sum::<u64>() / total) as u8
            })
        })
        .collect()
}

fn nearest(palette: &[Rgb], c: Rgb) -> u8 {
    let d = |p: &Rgb| -> u32 { (0..3).map(|i| (i32::from(p[i]) - i32::from(c[i])).pow(2) as u32).sum() };
    palette.iter().enumerate().min_by_key(|(_, p)| d(p)).map_or(0, |(i, _)| i as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_runs() {
        let mut out = Vec::new();
        rle(&[1, 1, 1, 1, 1, 0, 2, 2], &mut out);
        assert_eq!(out, b"!5@?AA");
    }

    #[test]
    fn small_palettes_are_exact() {
        let hist: HashMap<Rgb, u32> = [([1, 2, 3], 5), ([200, 0, 0], 1)].into_iter().collect();
        assert_eq!(median_cut(&hist, 256), vec![[1, 2, 3], [200, 0, 0]]);
    }

    #[test]
    fn median_cut_caps_the_palette() {
        let hist: HashMap<Rgb, u32> = (0..=255u8).flat_map(|r| [([r, 0, 0], 1), ([0, r, 255], 1)]).collect();
        let p = median_cut(&hist, 16);
        assert_eq!(p.len(), 16);
    }
}
