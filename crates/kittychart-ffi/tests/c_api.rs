//! Compile real C programs against `include/kittychart.h` and the static library, and run
//! them. Checks that the header matches the ABI and that C sees the same pixels as Rust.
//! Unix only: it needs `cc` (Windows runners have MSVC's `cl`).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// The static library cargo built next to this test (the `staticlib` crate type).
fn staticlib() -> PathBuf {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "-p", "kittychart-ffi"])
        .current_dir(dir())
        .status()
        .unwrap();
    assert!(status.success());
    let target = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).parent().unwrap().to_path_buf();
    target.join("debug/libkittychart_ffi.a")
}

/// Compile `src` with the system C compiler (`$CC`, else `cc`), warnings as errors.
pub fn compile(src: &Path, out: &Path) {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let mut cmd = Command::new(&cc);
    cmd.args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(out)
        .arg(src)
        .arg("-I")
        .arg(dir().join("include"))
        .arg(staticlib());
    if cfg!(target_os = "macos") {
        cmd.args(["-framework", "CoreFoundation"]);
    } else {
        cmd.args(["-lm", "-lpthread", "-ldl"]);
    }
    let o = cmd.output().unwrap_or_else(|e| panic!("{cc}: {e}"));
    assert!(o.status.success(), "{cc} failed:\n{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn c_program_uses_the_whole_api() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ffi");
    std::fs::create_dir_all(&tmp).unwrap();
    let exe = tmp.join("smoke");
    compile(&dir().join("tests/c/smoke.c"), &exe);
    let png = tmp.join("smoke.png");
    let o = Command::new(&exe).arg(&png).output().unwrap();
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ok");

    // The same pixels as rendering in Rust.
    let json = r#"{"type":"bar","data":{"labels":["Q1","Q2","Q3"],"datasets":[{"label":"Revenue","data":[12,19,7]}]}}"#;
    let expected = kittychart::render_json(json, &kittychart::RenderOptions::new(800.0)).unwrap();
    assert_eq!(std::fs::read(&png).unwrap(), expected.to_png());
}

#[test]
fn the_example_compiles() {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("ffi");
    std::fs::create_dir_all(&tmp).unwrap();
    compile(&dir().join("examples/show.c"), &tmp.join("show"));
}
