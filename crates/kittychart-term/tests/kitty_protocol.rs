//! L2 protocol tests: encode, then decode with the independent testkit decoder.
#![allow(clippy::unwrap_used)]

use kittychart_term::RgbaImage;
use kittychart_term::kitty::{self, MAX_CHUNK, Quiet, TransmitOptions};
use kittychart_testkit::kitty::{decode_transmissions, describe, parse_commands};
use proptest::prelude::*;

fn encode(w: u32, h: u32, px: &[u8], opts: &TransmitOptions) -> Vec<u8> {
    let mut out = Vec::new();
    kitty::transmit_and_display(RgbaImage::new(w, h, px), opts, &mut out);
    out
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Any image, compressed or not, round-trips exactly and obeys the chunking rules
    /// (the decoder enforces chunk size ≤ 4096, 4-aligned chunks, and continuation keys).
    #[test]
    fn round_trip(w in 1u32..96, h in 1u32..96, seed: u64, compress: bool, id in 1u32..u32::MAX) {
        let mut rng = seed | 1;
        let px: Vec<u8> = (0..w * h * 4).map(|_| { rng ^= rng << 7; rng ^= rng >> 9; rng as u8 }).collect();
        let mut opts = TransmitOptions::new(id);
        opts.compress = compress;
        let out = encode(w, h, &px, &opts);

        let decoded = decode_transmissions(&out).map_err(TestCaseError::fail)?;
        prop_assert_eq!(decoded.len(), 1);
        let t = &decoded[0];
        prop_assert_eq!((t.width, t.height, t.image_id), (w, h, Some(id)));
        prop_assert_eq!(&t.rgba, &px);
        prop_assert!(parse_commands(&out).iter().all(|c| c.payload.len() <= MAX_CHUNK));
    }
}

#[test]
fn chunk_boundaries_are_exact() {
    // Uncompressed payload sizes around the chunk boundary. 3072 raw bytes = 4096 base64 chars.
    for n_px in [767u32, 768, 769, 1536, 1537] {
        let px = vec![7u8; n_px as usize * 4];
        let mut o = TransmitOptions::new(1);
        o.compress = false;
        let out = encode(n_px, 1, &px, &o);
        let t = decode_transmissions(&out).unwrap();
        assert_eq!(t[0].rgba, px, "{n_px} px");
        let b64_len = (n_px as usize * 4).div_ceil(3) * 4;
        assert_eq!(t[0].chunks, b64_len.div_ceil(MAX_CHUNK), "{n_px} px");
    }
}

#[test]
fn snapshot_transmit_with_all_keys() {
    let px = vec![0u8; 64 * 48 * 4];
    let mut o = TransmitOptions::new(42);
    o.compress = false;
    o.placement_id = Some(3);
    o.columns = Some(8);
    o.rows = Some(3);
    o.hold_cursor = true;
    o.quiet = Quiet::ErrorsOnly;
    let mut out = encode(64, 48, &px, &o);
    kitty::delete_image(42, Quiet::Silent, &mut out);
    insta::assert_snapshot!(describe(&out));
}
