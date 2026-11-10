//! L1 golden tests, one per `tests/fixtures/**/*.json` (auto-discovered).
//!
//! A fixture is a Chart.js config. An optional `"_kc"` key sets render options:
//! `{"width": 512, "height": 256, "dpr": 1, "background": "#fff"}`. The golden is
//! `tests/goldens/fixtures/<path without .json>.png`. Bless with `KC_BLESS=1`.

use std::path::Path;

use kittychart::{RenderOptions, render_json};
use kittychart_testkit::golden::{Image, assert_golden};

fn fixture(path: &Path, contents: String) -> datatest_stable::Result<()> {
    let v: serde_json::Value = serde_json::from_str(&contents)?;
    let opts = RenderOptions::from_fixture(&v["_kc"]);
    let img = render_json(&contents, &opts)?;
    let root = kittychart_testkit::workspace_root().join("tests/fixtures");
    let rel = path.canonicalize()?.strip_prefix(&root)?.with_extension("");
    let name = format!("fixtures/{}", rel.to_string_lossy());
    assert_golden(&name, Image { width: img.width, height: img.height, rgba: &img.rgba });
    Ok(())
}

datatest_stable::harness! {
    { test = fixture, root = "../../tests/fixtures", pattern = r"\.json$" },
}
