//! Hit-testing parity with real Chart.js: `tests/interactions/*.json` holds what Chart.js 4.6.1's
//! `getElementsAtEventForMode` returns over a grid of positions, for every mode, with and
//! without `intersect` (recorded by `tools/reference/interactions.mjs`). Our port must agree
//! everywhere, including where nothing is hit.

use std::collections::HashMap;

use kittychart::canvas::SkiaCanvas;
use kittychart::{Chart, ChartConfig};
use kittychart_core::interaction::{ActiveElement, InteractionOptions, Mode};
use serde_json::Value;

#[test]
fn hit_testing_matches_chartjs() {
    let root = kittychart_testkit::workspace_root();
    let dir = root.join("tests/interactions");
    let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    files.sort();
    assert!(files.len() >= 10);
    let mut mismatches = Vec::new();
    let mut checked = 0;
    for file in files {
        let rec: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let fixture = file.file_name().unwrap().to_string_lossy().replacen('-', "/", 1);
        let json = std::fs::read_to_string(root.join("tests/fixtures").join(&fixture)).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        let dpr = v["_kc"]["dpr"].as_f64().unwrap_or(1.0) as f32;
        let (w, h, step) =
            (rec["width"].as_f64().unwrap(), rec["height"].as_f64().unwrap(), rec["step"].as_f64().unwrap());
        let mut chart = Chart::new(ChartConfig::from_json(&json).unwrap());
        chart.dpr = f64::from(dpr);
        chart.draw(&mut SkiaCanvas::new(w as f32, h as f32, dpr));

        let mut expected: HashMap<(String, bool, i64, i64), Vec<ActiveElement>> = HashMap::new();
        for q in rec["queries"].as_array().unwrap() {
            let els = q[4].as_array().unwrap().iter().map(|e| ActiveElement {
                dataset: e[0].as_u64().unwrap() as usize,
                index: e[1].as_u64().unwrap() as usize,
            });
            let key = (
                q[0].as_str().unwrap().to_owned(),
                q[1].as_bool().unwrap(),
                q[2].as_f64().unwrap() as i64,
                q[3].as_f64().unwrap() as i64,
            );
            expected.insert(key, els.collect());
        }
        for mode in ["index", "dataset", "point", "nearest", "x", "y"] {
            for intersect in [true, false] {
                let opts =
                    InteractionOptions { mode: Mode::parse(mode).unwrap(), intersect, ..Default::default() };
                let mut y = step / 2.0;
                while y < h {
                    let mut x = step / 2.0;
                    while x < w {
                        let ours = chart.elements_at(x, y, &opts);
                        let theirs = expected
                            .get(&(mode.to_owned(), intersect, x as i64, y as i64))
                            .cloned()
                            .unwrap_or_default();
                        checked += 1;
                        if ours != theirs && mismatches.len() < 20 {
                            mismatches.push(format!("{fixture} {mode} intersect={intersect} at ({x}, {y}): ours {ours:?}, Chart.js {theirs:?}"));
                        }
                        x += step;
                    }
                    y += step;
                }
            }
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} of {checked} queries differ:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!("{checked} queries match Chart.js");
}

/// `Chart::hover_at` (what the viewer, the widgets and the browser demo call on pointer
/// motion): the hover mode picks the active elements, the tooltip mode what the tooltip lists,
/// and it reports a change only when the next draw would differ.
#[test]
fn hover_at_sets_active_and_tooltip() {
    let json = r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[
        {"data":[3,5,2]},{"data":[4,1,6]}]},
        "options":{"animation":false,"interaction":{"mode":"index","intersect":false},
        "plugins":{"tooltip":{"mode":"nearest"}}}}"#;
    let mut chart = Chart::new(ChartConfig::from_json(json).unwrap());
    chart.draw(&mut SkiaCanvas::new(600.0, 400.0, 1.0));
    // Over the middle category: index mode activates both datasets' "b" bars.
    let x = 300.0;
    assert!(chart.hover_at(Some((x, 200.0))));
    let both = vec![ActiveElement { dataset: 0, index: 1 }, ActiveElement { dataset: 1, index: 1 }];
    assert_eq!(chart.active(), both.as_slice());
    let expected_tooltip = chart.elements_at(x, 200.0, &chart.tooltip_interaction());
    assert_eq!(expected_tooltip.len(), 1, "nearest mode lists one element");
    // The same position again: nothing changes.
    assert!(!chart.hover_at(Some((x, 200.0))));
    // A new position over the same elements still changes the tooltip anchor.
    assert!(chart.hover_at(Some((x + 1.0, 200.0))));
    // Leaving clears everything, once.
    assert!(chart.hover_at(None));
    assert!(chart.active().is_empty());
    assert!(!chart.hover_at(None));
}
