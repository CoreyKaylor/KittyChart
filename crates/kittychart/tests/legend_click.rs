//! Legend clicks: hit boxes from the drawn legend, and Chart.js's default onClick behaviour.

use kittychart::canvas::SkiaCanvas;
use kittychart::{Chart, ChartConfig};

fn draw(chart: &mut Chart) {
    chart.draw(&mut SkiaCanvas::new(512.0, 256.0, 1.0));
}

fn center(b: [f64; 4]) -> (f64, f64) {
    (b[0] + b[2] / 2.0, b[1] + b[3] / 2.0)
}

#[test]
fn clicking_a_dataset_item_hides_and_shows_it() {
    let json = r#"{"type":"bar","data":{"labels":["a","b"],"datasets":[
        {"label":"One","data":[1,2]},{"label":"Two","data":[3,4]}]}}"#;
    let mut chart = Chart::new(ChartConfig::from_json(json).unwrap());
    draw(&mut chart);
    let boxes = chart.legend.item_boxes();
    assert_eq!(boxes.len(), 2);
    let (x, y) = center(boxes[1]);
    assert_eq!(chart.legend_item_at(x, y).map(|i| i.dataset_index), Some(1));
    assert!(chart.legend_item_at(x, y + 100.0).is_none(), "below the legend");

    assert!(chart.legend_click(x, y));
    assert!(!chart.dataset_visible(1));
    draw(&mut chart);
    assert!(chart.legend.items[1].hidden, "struck through");
    // The y scale now fits dataset 0 only (max 2, not 4).
    assert!(chart.scales.iter().any(|s| s.id == "y" && s.max < 3.0));

    let (x, y) = center(chart.legend.item_boxes()[1]);
    assert!(chart.legend_click(x, y));
    assert!(chart.dataset_visible(1), "a second click shows it again");
}

#[test]
fn clicking_a_doughnut_item_toggles_that_slice() {
    let json = r#"{"type":"doughnut","data":{"labels":["a","b","c"],"datasets":[{"data":[1,1,2]}]}}"#;
    let mut chart = Chart::new(ChartConfig::from_json(json).unwrap());
    draw(&mut chart);
    let (x, y) = center(chart.legend.item_boxes()[2]);
    assert!(chart.legend_click(x, y));
    draw(&mut chart);
    let arcs = &chart.metas[0].arcs;
    assert_eq!(arcs[2].circumference, 0.0);
    assert!((arcs[0].circumference - std::f64::consts::PI).abs() < 1e-9, "the rest fill the circle");
}
