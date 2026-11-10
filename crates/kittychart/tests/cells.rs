//! Text-fallback snapshots: the characters a chart becomes in a terminal without graphics.

use kittychart::{ChartConfig, render_cells};
use kittychart_canvas::cells::CellMode;

fn plain(path: &str, cols: u32, rows: u32) -> String {
    let json = std::fs::read_to_string(kittychart_testkit::workspace_root().join(path)).unwrap();
    render_cells(ChartConfig::from_json(&json).unwrap(), cols, rows, CellMode::Auto, None).to_plain()
}

#[test]
fn bar_chart_as_cells() {
    insta::assert_snapshot!(plain("tests/fixtures/bar/grouped.json", 72, 18));
}

#[test]
fn line_chart_as_cells() {
    insta::assert_snapshot!(plain("tests/fixtures/line/stepped_monotone_points.json", 72, 18));
}
