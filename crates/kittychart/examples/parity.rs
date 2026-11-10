//! Parity report: KittyChart vs. real Chart.js for every fixture.
//!
//! Run via `just parity`. It renders references with `tools/reference/render.mjs`, then this
//! renders ours, scores each pair with DSSIM, and writes `target/parity/report.html`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use kittychart::{RenderOptions, render_json};
use kittychart_testkit::golden::{Image, read_png, write_png};

fn main() {
    let root = kittychart_testkit::workspace_root();
    let fixtures = root.join("tests/fixtures");
    let out = root.join("target/parity");
    let mut rows = Vec::new();
    for file in walk(&fixtures) {
        let rel = file.strip_prefix(&fixtures).unwrap().with_extension("png");
        let json = std::fs::read_to_string(&file).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let opts = RenderOptions::from_fixture(&v["_kc"]);
        let ours = match render_json(&json, &opts) {
            Ok(i) => i,
            Err(e) => {
                rows.push((rel.clone(), format!("error: {e}"), None));
                continue;
            }
        };
        let ours_path = out.join("ours").join(&rel);
        std::fs::create_dir_all(ours_path.parent().unwrap()).unwrap();
        write_png(&ours_path, Image { width: ours.width, height: ours.height, rgba: &ours.rgba });

        let ref_path = out.join("ref").join(&rel);
        if !ref_path.exists() {
            rows.push((rel, "no reference".into(), None));
            continue;
        }
        let (rw, rh, rdata) = read_png(&ref_path);
        if (rw, rh) != (ours.width, ours.height) {
            rows.push((
                rel,
                format!("size differs: ref {rw}x{rh}, ours {}x{}", ours.width, ours.height),
                None,
            ));
            continue;
        }
        let score = dssim(ours.width, ours.height, &ours.rgba, &rdata);
        rows.push((rel, String::new(), Some(score)));
    }
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut html = String::from(
        "<!doctype html><meta charset=utf-8><title>KittyChart parity</title><style>\
         body{font:14px system-ui;margin:16px;background:#f6f6f6}td{vertical-align:top;padding:6px}\
         img{max-width:420px;border:1px solid #ddd;background:#fff}th{text-align:left}</style>\
         <h1>KittyChart vs Chart.js</h1><p>DSSIM: 0 = identical. Sorted worst first.</p>\
         <table><tr><th>fixture</th><th>Chart.js</th><th>KittyChart</th></tr>",
    );
    println!("{:<40} dssim", "fixture");
    for (rel, note, score) in &rows {
        let r = rel.to_string_lossy();
        let s = score.map_or(note.clone(), |s| format!("{s:.5}"));
        println!("{r:<40} {s}");
        let _ = write!(
            html,
            "<tr><td><b>{r}</b><br>{s}</td><td><img src=\"ref/{r}\"></td><td><img src=\"ours/{r}\"></td></tr>"
        );
    }
    html.push_str("</table>");
    std::fs::write(out.join("report.html"), html).unwrap();
    println!("\nreport: {}", out.join("report.html").display());
}

fn dssim(w: u32, h: u32, a: &[u8], b: &[u8]) -> f64 {
    let d = dssim_core::Dssim::new();
    let img = |px: &[u8]| {
        let v: Vec<rgb::RGBA8> =
            px.as_chunks::<4>().0.iter().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect();
        d.create_image_rgba(&v, w as usize, h as usize).unwrap()
    };
    f64::from(d.compare(&img(a), img(b)).0)
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "json") {
            out.push(p);
        }
    }
    out.sort();
    out
}
