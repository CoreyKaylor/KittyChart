//! SVG output, checked by an independent renderer: every fixture is rendered to SVG,
//! rasterized by resvg, and compared with the PNG renderer's output.

use std::path::Path;

use kittychart::{ChartConfig, RenderOptions, render, render_svg};
use kittychart_testkit::golden::{Image, dssim};

fn fixtures(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            fixtures(&p, out);
        } else if p.extension().is_some_and(|e| e == "json") {
            out.push(p);
        }
    }
}

fn rasterize(svg: &str, w: u32, h: u32) -> Vec<u8> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("valid SVG");
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    // resvg's pixmap is premultiplied; the charts here have opaque backgrounds.
    pixmap.data().to_vec()
}

#[test]
fn every_fixture_renders_the_same_as_svg() {
    let root = kittychart_testkit::workspace_root().join("tests/fixtures");
    let mut files = Vec::new();
    fixtures(&root, &mut files);
    assert!(files.len() > 20);
    let mut worst = (0.0, String::new());
    for file in files {
        let json = std::fs::read_to_string(&file).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let kc = &v["_kc"];
        let num = |k: &str, d: f64| kc.get(k).and_then(serde_json::Value::as_f64).unwrap_or(d) as f32;
        let mut opts = RenderOptions::new(num("width", 512.0)).dpr(num("dpr", 1.0));
        opts.height = kc.get("height").and_then(serde_json::Value::as_f64).map(|h| h as f32);
        opts.background =
            kittychart::parse_color(kc.get("background").and_then(|b| b.as_str()).unwrap_or("#ffffff"));
        let config = ChartConfig::from_json(&json).unwrap();
        let png = render(config.clone(), &opts);
        let svg = render_svg(config, &opts);
        let raster = rasterize(&svg, png.width, png.height);
        let score = dssim(
            Image { width: png.width, height: png.height, rgba: &png.rgba },
            Image { width: png.width, height: png.height, rgba: &raster },
        );
        let name = file.strip_prefix(&root).unwrap().display().to_string();
        if score > 0.002 {
            let dir = kittychart_testkit::workspace_root().join("target/kc-diffs/svg");
            std::fs::create_dir_all(&dir).unwrap();
            let base = name.replace('/', "-").replace(".json", "");
            std::fs::write(dir.join(format!("{base}.svg")), &svg).unwrap();
            kittychart_testkit::golden::write_png(
                &dir.join(format!("{base}-resvg.png")),
                Image { width: png.width, height: png.height, rgba: &raster },
            );
            kittychart_testkit::golden::write_png(
                &dir.join(format!("{base}-png.png")),
                Image { width: png.width, height: png.height, rgba: &png.rgba },
            );
        }
        if score > worst.0 {
            worst = (score, name.clone());
        }
        assert!(score < 0.002, "{name}: SVG rasterized by resvg differs from the PNG (dssim {score:.5})");
    }
    eprintln!("worst fixture: {} (dssim {:.5})", worst.1, worst.0);
}

#[test]
fn svg_is_self_contained() {
    let json = r#"{"type":"bar","data":{"labels":["a"],"datasets":[{"label":"x","data":[1]}]}}"#;
    let svg = render_svg(ChartConfig::from_json(json).unwrap(), &RenderOptions::new(300.0));
    assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
    assert!(!svg.contains("<text") && !svg.contains("<image"), "text is outlines, nothing external");
}
