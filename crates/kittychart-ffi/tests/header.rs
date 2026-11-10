//! `include/kittychart.h` is generated from the Rust source by cbindgen. This test fails when
//! it's stale; `KC_BLESS=1` (or `just header`) rewrites it.

use std::path::Path;

#[test]
fn header_is_current() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = cbindgen::Config::from_file(dir.join("cbindgen.toml")).unwrap();
    let mut generated = Vec::new();
    cbindgen::Builder::new()
        .with_crate(dir)
        .with_config(config)
        .generate()
        .expect("cbindgen")
        .write(&mut generated);
    let path = dir.join("include/kittychart.h");
    if std::env::var_os("KC_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let current = std::fs::read(&path).unwrap_or_default();
    assert!(
        current == generated,
        "include/kittychart.h is stale: run `just header` (KC_BLESS=1 cargo test -p kittychart-ffi --test header)"
    );
}
