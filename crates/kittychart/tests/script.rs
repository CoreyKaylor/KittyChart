//! Scriptable options: functions (Rust closures here; bindings use the same markers) in place
//! of option values.

use kittychart::canvas::SkiaCanvas;
use kittychart::{Chart, ChartConfig, script};
use serde_json::{Value, json};

fn bar(options: Value, dataset: Value) -> ChartConfig {
    let mut ds = json!({ "label": "Sales", "data": [5, 15, 8] });
    for (k, v) in dataset.as_object().unwrap() {
        ds[k] = v.clone();
    }
    ChartConfig::from_value(json!({
        "type": "bar",
        "data": { "labels": ["a", "b", "c"], "datasets": [ds] },
        "options": options,
    }))
    .unwrap()
}

/// Draw at 1:1 and return the chart (with its laid-out elements) and the pixels.
fn draw(config: ChartConfig) -> (Chart, SkiaCanvas) {
    let mut chart = Chart::new(config);
    let mut canvas = SkiaCanvas::new(400.0, 200.0, 1.0);
    canvas.fill_background(kittychart::Color::rgba8(255, 255, 255, 255));
    chart.draw(&mut canvas);
    (chart, canvas)
}

fn pixel(c: &SkiaCanvas, x: f64, y: f64) -> [u8; 3] {
    let rgba = c.to_rgba();
    let i = ((y as usize) * c.pixel_width() as usize + x as usize) * 4;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

#[test]
fn per_element_colors_from_a_function() {
    let config = bar(json!({}), json!({ "backgroundColor": script::marker("color"), "borderWidth": 0 }))
        .with_fn("color", |ctx| {
            // The data context: the raw value, the index and the dataset.
            assert_eq!(ctx["type"], "data");
            assert_eq!(ctx["dataset"]["label"], "Sales");
            assert!(ctx["dataset"].get("data").is_none(), "the dataset context leaves the data out");
            let raw = ctx["raw"].as_f64().ok_or("no raw")?;
            assert_eq!(ctx["parsed"]["y"].as_f64(), Some(raw));
            Ok(json!(if raw > 10.0 { "rgb(255, 0, 0)" } else { "rgb(0, 0, 255)" }))
        });
    let (chart, canvas) = draw(config);
    let colors: Vec<[u8; 3]> =
        chart.metas[0].bars.iter().map(|b| pixel(&canvas, b.x, (b.y + b.base) / 2.0)).collect();
    assert_eq!(colors, [[0, 0, 255], [255, 0, 0], [0, 0, 255]]);
}

#[test]
fn tick_callback_gets_value_index_and_ticks() {
    let config =
        bar(json!({ "scales": { "y": { "ticks": { "callback": script::marker("money") } } } }), json!({}))
            .with_fn("money", |ctx| {
                let args = ctx["__args"].as_array().ok_or("no args")?;
                let (value, index, ticks) = (&args[0], &args[1], &args[2]);
                assert_eq!(ticks[index.as_u64().unwrap() as usize]["value"], *value);
                Ok(json!(format!("${}", value)))
            });
    let mut chart = Chart::for_cells(config, None);
    let mut cells = kittychart::canvas::cells::CellCanvas::new(60, 20, kittychart::CellMode::Auto);
    chart.draw(&mut cells);
    let text = cells.into_grid().to_plain();
    assert!(text.contains("$15") && text.contains("$0"), "{text}");
}

#[test]
fn undefined_falls_through_and_errors_are_collected() {
    let config = bar(
        json!({}),
        json!({ "borderWidth": script::marker("w"), "backgroundColor": script::marker("bad") }),
    )
    .with_fn("w", |_| Ok(Value::Null))
    .with_fn("bad", |_| Err("boom".into()));
    let script = config.script.clone().unwrap();
    let (chart, _) = draw(config);
    let bar = &chart.metas[0].bars[0];
    assert_eq!(bar.options.border_width.top, 0.0, "null falls through to the bar default (0)");
    let errors = script.take_errors();
    assert_eq!(errors, ["function \"bad\": boom"]);
}

#[test]
fn scriptable_objects_and_chart_context() {
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let config = bar(
        json!({
            "plugins": { "title": { "display": true, "text": "Scripted", "font": script::marker("font") } }
        }),
        json!({}),
    )
    .with_fn("font", {
        let seen = seen.clone();
        move |ctx| {
            assert_eq!(ctx["type"], "chart");
            seen.lock().unwrap().push(ctx["chart"]["width"].as_f64().unwrap_or(-1.0));
            Ok(json!({ "size": 30, "weight": "bold" }))
        }
    });
    let (chart, _) = draw(config);
    assert_eq!(seen.lock().unwrap().last(), Some(&400.0), "drawing resolves with the chart's size");
    assert_eq!(chart.title.font.size, 30.0);
    assert_eq!(chart.title.font.weight, Some(700));
}

#[test]
fn without_functions_nothing_changes() {
    let plain = bar(json!({}), json!({ "backgroundColor": "rgb(0, 128, 0)" }));
    let with_unused_fn = plain.clone().with_fn("unused", |_| Ok(json!("red")));
    assert_eq!(draw(plain).1.to_rgba(), draw(with_unused_fn).1.to_rgba());
}

#[test]
fn segment_styles_from_a_function() {
    // Chart.js's segment example: falling segments red, rising green.
    let config = ChartConfig::from_value(json!({
        "type": "line",
        "data": { "labels": ["a", "b", "c", "d"], "datasets": [{
            "data": [1, 5, 2, 6],
            "borderWidth": 6,
            "pointRadius": 0,
            "segment": { "borderColor": script::marker("trend") },
        }]},
        "options": { "plugins": { "legend": { "display": false } } },
    }))
    .unwrap()
    .with_fn("trend", |ctx| {
        assert_eq!(ctx["type"], "segment");
        let (y0, y1) =
            (ctx["p0"]["parsed"]["y"].as_f64().unwrap(), ctx["p1"]["parsed"]["y"].as_f64().unwrap());
        assert_eq!(ctx["p1DataIndex"].as_u64().unwrap(), ctx["p0DataIndex"].as_u64().unwrap() + 1);
        Ok(json!(if y1 < y0 { "rgb(255, 0, 0)" } else { "rgb(0, 160, 0)" }))
    });
    let (chart, canvas) = draw(config);
    let pts = &chart.metas[0].points;
    let mid = |a: usize| pixel(&canvas, (pts[a].x + pts[a + 1].x) / 2.0, (pts[a].y + pts[a + 1].y) / 2.0);
    assert_eq!([mid(0), mid(1), mid(2)], [[0, 160, 0], [255, 0, 0], [0, 160, 0]]);
    let line = chart.metas[0].line.as_ref().unwrap();
    assert_eq!(line.styled.as_ref().map(Vec::len), Some(3), "split into three styled segments");
}
