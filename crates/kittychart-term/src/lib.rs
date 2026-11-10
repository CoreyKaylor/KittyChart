//! Terminal output for KittyChart.
//!
//! Everything in this crate is **sans-IO**: encoders turn pixels into bytes, and the
//! [`detect::Detector`] turns terminal replies into capabilities. Nothing here reads a file
//! descriptor, an environment variable, or a clock. That keeps every behavior testable with plain
//! byte slices, or against an in-process terminal emulator (see `kittychart-conformance`).

pub mod detect;
pub mod kitty;
#[cfg(all(unix, feature = "tty"))]
pub mod tty;

/// A borrowed, tightly packed, non-premultiplied RGBA8 image.
#[derive(Debug, Clone, Copy)]
pub struct RgbaImage<'a> {
    pub width: u32,
    pub height: u32,
    pub data: &'a [u8],
}

impl<'a> RgbaImage<'a> {
    /// # Panics
    /// If `data.len() != width * height * 4`.
    pub fn new(width: u32, height: u32, data: &'a [u8]) -> Self {
        assert_eq!(
            data.len(),
            width as usize * height as usize * 4,
            "RGBA buffer length does not match {width}x{height}"
        );
        Self { width, height, data }
    }
}
