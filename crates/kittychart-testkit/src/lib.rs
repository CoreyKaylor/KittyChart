//! Test support for KittyChart.
//!
//! - [`kitty`]: an independent decoder for kitty graphics escapes. Encoder tests round-trip
//!   through it, so they never need a terminal.
//! - [`sixel`]: an independent sixel decoder.
//! - [`golden`]: golden-image assertions with a perceptual tolerance, blessing, and diff output.

pub mod golden;
pub mod kitty;
pub mod sixel;

use std::path::PathBuf;

/// Workspace root, resolved at compile time and independent of the test's working directory.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("workspace root")
}
