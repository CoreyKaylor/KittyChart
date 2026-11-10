//! Sixel round trips through the testkit's independent decoder.

use kittychart_term::RgbaImage;
use kittychart_term::sixel::{SixelOptions, encode};
use kittychart_testkit::sixel::decode;
use proptest::prelude::*;

fn roundtrip(w: u32, h: u32, rgba: &[u8], opts: &SixelOptions) -> kittychart_testkit::sixel::Decoded {
    let mut out = Vec::new();
    encode(RgbaImage::new(w, h, rgba), opts, &mut out);
    decode(&out)
}

/// Sixel palettes are in percent, so each channel may be off by up to 1.3 levels.
fn close(a: &[u8], b: &[u8]) -> bool {
    a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 2)
}

#[test]
fn transparent_pixels_stay_unset() {
    // 3×7 (crosses a band boundary): opaque red column, transparent elsewhere.
    let mut rgba = vec![0u8; 3 * 7 * 4];
    for y in 0..7 {
        rgba[(y * 3) * 4..(y * 3) * 4 + 4].copy_from_slice(&[255, 0, 0, 255]);
    }
    let d = roundtrip(3, 7, &rgba, &SixelOptions::default());
    assert!(d.transparent_background);
    assert_eq!((d.width, d.height), (3, 7));
    assert_eq!(d.rgba, rgba);
}

#[test]
fn translucent_pixels_blend_onto_the_background() {
    let rgba = [255, 255, 255, 128];
    let d = roundtrip(1, 1, &rgba, &SixelOptions { background: Some([0, 0, 0]), ..SixelOptions::default() });
    assert!(close(&d.rgba, &[128, 128, 128, 255]), "{:?}", d.rgba);
}

proptest! {
    #[test]
    fn few_color_images_roundtrip(
        w in 1u32..40,
        h in 1u32..20,
        palette in proptest::collection::vec(any::<[u8; 3]>(), 1..12),
        seed in proptest::collection::vec(any::<u16>(), 800),
    ) {
        let rgba: Vec<u8> = (0..(w * h) as usize)
            .flat_map(|i| {
                let s = seed[i % seed.len()] as usize;
                if s.is_multiple_of(5) { [0, 0, 0, 0] } else { let c = palette[s % palette.len()]; [c[0], c[1], c[2], 255] }
            })
            .collect();
        let d = roundtrip(w, h, &rgba, &SixelOptions::default());
        prop_assert_eq!((d.width, d.height), (w, h));
        prop_assert!(close(&d.rgba, &rgba));
    }

    #[test]
    fn many_colors_stay_close(w in 8u32..64, h in 8u32..64) {
        // A smooth gradient: more distinct colors than palette entries.
        let rgba: Vec<u8> = (0..h).flat_map(|y| (0..w).flat_map(move |x| {
            [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) * 127 / (w + h)) as u8, 255]
        })).collect();
        let d = roundtrip(w, h, &rgba, &SixelOptions { max_colors: 64, ..SixelOptions::default() });
        let err = d.rgba.iter().zip(&rgba).map(|(a, b)| u32::from(a.abs_diff(*b))).max().unwrap();
        prop_assert!(err <= 48, "max channel error {}", err);
    }
}
