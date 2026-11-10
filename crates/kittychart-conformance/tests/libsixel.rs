//! Sixel output decoded by libsixel (`sixel2png`), the reference implementation most
//! terminals' sixel support is tested against. Our own testkit decoder can share our reading
//! of the spec; this one can't. Needs libsixel's tools (`apt install libsixel-bin`,
//! `pacman -S libsixel`, `brew install libsixel`).

use std::path::PathBuf;
use std::process::Command;

use kittychart::terminal::{Protocol, TermGeometry, TermOptions, render_for_terminal};
use kittychart_term::RgbaImage;
use kittychart_term::sixel::{SixelOptions, encode};

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("libsixel");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Decode with libsixel. Pixels left unset (transparent) come back as libsixel's background.
fn sixel2png(name: &str, sixel: &[u8]) -> (u32, u32, Vec<u8>) {
    let (six, png) = (scratch(&format!("{name}.six")), scratch(&format!("{name}.png")));
    std::fs::write(&six, sixel).unwrap();
    let status = Command::new("sixel2png")
        .arg("-i")
        .arg(&six)
        .arg("-o")
        .arg(&png)
        .status()
        .expect("sixel2png not found: install libsixel (libsixel-bin on Debian/Ubuntu)");
    assert!(status.success(), "sixel2png rejected {}", six.display());
    kittychart_testkit::golden::read_png(&png)
}

/// Peak signal-to-noise ratio over RGB.
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let (mut se, mut n) = (0.0, 0.0);
    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        for i in 0..3 {
            se += (f64::from(pa[i]) - f64::from(pb[i])).powi(2);
            n += 1.0;
        }
    }
    if se == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64.powi(2) / (se / n)).log10() }
}

#[test]
fn exact_for_few_colors() {
    // 3×13: crosses two band boundaries; three colors in stripes.
    let colors = [[220u8, 40, 40], [40, 200, 60], [30, 60, 230]];
    let rgba: Vec<u8> = (0..13)
        .flat_map(|y| {
            (0..3).flat_map(move |x| {
                let c = colors[(x + y) % 3];
                [c[0], c[1], c[2], 255]
            })
        })
        .collect();
    let mut out = Vec::new();
    encode(RgbaImage::new(3, 13, &rgba), &SixelOptions::default(), &mut out);
    let (w, h, px) = sixel2png("stripes", &out);
    assert_eq!((w, h), (3, 13));
    // Percent-precision palette: within 2 levels per channel.
    for (i, (a, b)) in px.as_chunks::<4>().0.iter().zip(rgba.as_chunks::<4>().0).enumerate() {
        assert!(a[..3].iter().zip(&b[..3]).all(|(x, y)| x.abs_diff(*y) <= 2), "pixel {i}: {a:?} vs {b:?}");
    }
}

#[test]
fn charts_survive_quantization() {
    // Opaque background: every pixel is encoded (no transparency to fill on decode).
    for fixture in ["bar/grouped", "line/basic", "arc/pie", "fill/stacked_area"] {
        let path = kittychart_testkit::workspace_root().join(format!("tests/fixtures/{fixture}.json"));
        let config = kittychart::ChartConfig::from_json(&std::fs::read_to_string(path).unwrap()).unwrap();
        let geom = TermGeometry { cols: 60, cell_width: 10.0, cell_height: 20.0 };
        let opts = TermOptions {
            protocol: Protocol::Sixel,
            background: Some(kittychart::Color::rgba8(0x1a, 0x1b, 0x26, 255)),
            ..TermOptions::default()
        };
        let r = render_for_terminal(config, geom, &opts);
        let (w, h, px) = sixel2png(&fixture.replace('/', "-"), &r.bytes);
        assert_eq!((w, h), (r.image.width, r.image.height), "{fixture}");
        let score = psnr(&px, &r.image.rgba);
        assert!(score > 45.0, "{fixture}: PSNR {score:.1} dB");
    }
}

#[test]
fn transparent_pixels_stay_unset() {
    // 4×6: left half transparent; right half red, except one blue pixel. A transparent pixel
    // encoded as anything but "unset" would come back as one of the palette's colors.
    let rgba: Vec<u8> = (0..6)
        .flat_map(|y| {
            (0..4).flat_map(move |x| match (x, y) {
                (0 | 1, _) => [0, 0, 0, 0],
                (3, 5) => [0, 0, 255, 255],
                _ => [255, 0, 0, 255],
            })
        })
        .collect();
    let mut out = Vec::new();
    encode(RgbaImage::new(4, 6, &rgba), &SixelOptions::default(), &mut out);
    assert!(out.starts_with(b"\x1bP0;1;0q"), "P2=1: unset pixels keep the background");
    let (w, h, px) = sixel2png("transparent", &out);
    assert_eq!((w, h), (4, 6));
    let at = |x: usize, y: usize| &px[(y * 4 + x) * 4..(y * 4 + x) * 4 + 3];
    let drawn = [[255u8, 0, 0], [0, 0, 255]];
    for y in 0..6 {
        for x in 0..2 {
            assert!(!drawn.iter().any(|c| at(x, y) == c), "({x},{y}) was drawn: {:?}", at(x, y));
        }
        assert_eq!(at(2, y), &[255, 0, 0]);
    }
    assert_eq!(at(3, 5), &[0, 0, 255]);
}
