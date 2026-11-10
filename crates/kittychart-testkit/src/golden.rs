//! Golden-image assertions.
//!
//! Goldens live in `<workspace>/tests/goldens/<name>.png`. The compare runs in three steps:
//! 1. Identical bytes pass immediately (the common case on the dev platform).
//! 2. Otherwise compute DSSIM, which tolerates the sub-pixel anti-aliasing differences that show
//!    up across CPU architectures and SIMD paths.
//! 3. On failure, write `actual.png`, `expected.png` and `diff.png` under `target/kc-diffs/<name>/`.
//!
//! Set `KC_BLESS=1` to (re)write goldens. A missing golden is a failure unless blessing, so CI
//! can never silently create one.

use std::fs;
use std::path::{Path, PathBuf};

use crate::workspace_root;

/// Default DSSIM threshold. 0 means identical. Anti-aliasing jitter scores around 1e-5, and a
/// visibly moved line or changed color scores above 1e-3.
pub const DEFAULT_THRESHOLD: f64 = 2e-4;

#[derive(Debug, Clone, Copy)]
pub struct Image<'a> {
    pub width: u32,
    pub height: u32,
    /// Non-premultiplied RGBA8.
    pub rgba: &'a [u8],
}

pub fn golden_path(name: &str) -> PathBuf {
    workspace_root().join("tests/goldens").join(format!("{name}.png"))
}

/// Compare `img` against the golden `name` with [`DEFAULT_THRESHOLD`].
#[track_caller]
pub fn assert_golden(name: &str, img: Image<'_>) {
    assert_golden_with(name, img, DEFAULT_THRESHOLD);
}

#[track_caller]
pub fn assert_golden_with(name: &str, img: Image<'_>, threshold: f64) {
    let path = golden_path(name);
    let bless = std::env::var_os("KC_BLESS").is_some_and(|v| v != "0");

    let expected = path.exists().then(|| read_png(&path));
    match expected {
        Some(ref e) if e.0 == img.width && e.1 == img.height && e.2 == img.rgba => return,
        None if !bless => {
            let dir = write_diffs(name, img, None);
            panic!(
                "golden {name:?} does not exist ({}).\nActual image written to {}.\n\
                 Re-run with KC_BLESS=1 to accept it.",
                path.display(),
                dir.display()
            );
        }
        _ => {}
    }

    if let Some((w, h, ref data)) = expected {
        if (w, h) == (img.width, img.height) {
            let score = dssim(img, Image { width: w, height: h, rgba: data });
            if score <= threshold {
                return;
            }
            if !bless {
                let dir = write_diffs(name, img, Some((w, h, data)));
                panic!(
                    "golden {name:?} differs: dssim {score:.6} > threshold {threshold}.\n\
                     See {}/{{actual,expected,diff}}.png. Re-run with KC_BLESS=1 to accept.",
                    dir.display()
                );
            }
        } else if !bless {
            let dir = write_diffs(name, img, None);
            panic!(
                "golden {name:?} is {w}x{h} but actual is {}x{}. See {}. KC_BLESS=1 to accept.",
                img.width,
                img.height,
                dir.display()
            );
        }
    }

    // Blessing.
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    write_png(&path, img);
    eprintln!("blessed golden {}", path.display());
}

/// DSSIM between two same-size images (0 = identical).
pub fn dssim(a: Image<'_>, b: Image<'_>) -> f64 {
    let d = dssim_core::Dssim::new();
    let to_img = |i: Image<'_>| {
        let px: Vec<rgb::RGBA8> =
            i.rgba.as_chunks::<4>().0.iter().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
        d.create_image_rgba(&px, i.width as usize, i.height as usize).expect("dssim image")
    };
    let (val, _) = d.compare(&to_img(a), to_img(b));
    f64::from(val)
}

fn diff_dir(name: &str) -> PathBuf {
    let target =
        std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| workspace_root().join("target"), PathBuf::from);
    target.join("kc-diffs").join(name)
}

fn write_diffs(name: &str, actual: Image<'_>, expected: Option<(u32, u32, &[u8])>) -> PathBuf {
    let dir = diff_dir(name);
    fs::create_dir_all(&dir).unwrap();
    write_png(&dir.join("actual.png"), actual);
    if let Some((w, h, e)) = expected {
        write_png(&dir.join("expected.png"), Image { width: w, height: h, rgba: e });
        // Diff: dimmed grayscale of the expected image, with changed pixels in magenta.
        let diff: Vec<u8> = e
            .as_chunks::<4>()
            .0
            .iter()
            .zip(actual.rgba.as_chunks::<4>().0)
            .flat_map(|(x, y)| {
                if x == y {
                    let g = ((u16::from(x[0]) + u16::from(x[1]) + u16::from(x[2])) / 3 / 3 + 170) as u8;
                    [g, g, g, 255]
                } else {
                    [255, 0, 255, 255]
                }
            })
            .collect();
        write_png(&dir.join("diff.png"), Image { width: w, height: h, rgba: &diff });
    }
    dir
}

pub fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let mut dec = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap()));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    assert_eq!(info.color_type, png::ColorType::Rgba, "golden {} must be RGBA8", path.display());
    (info.width, info.height, buf)
}

pub fn write_png(path: &Path, img: Image<'_>) {
    let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
    let mut enc = png::Encoder::new(file, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::High);
    enc.write_header().unwrap().write_image_data(img.rgba).unwrap();
}
