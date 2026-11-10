//! Print a fixture as terminal cells: `cargo run -p kittychart --example cells -- FILE [auto|blocks|half|braille] [cols] [rows]`
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let json = std::fs::read_to_string(&a[1]).unwrap();
    let mode = match a.get(2).map(String::as_str) {
        Some("braille") => kittychart_canvas::cells::CellMode::Braille,
        Some("half") => kittychart_canvas::cells::CellMode::HalfBlocks,
        Some("blocks") => kittychart_canvas::cells::CellMode::Blocks,
        _ => kittychart_canvas::cells::CellMode::Auto,
    };
    let cols = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(80);
    let rows = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(20);
    let grid =
        kittychart::render_cells(kittychart::ChartConfig::from_json(&json).unwrap(), cols, rows, mode, None);
    println!("{}", grid.to_ansi());
}
