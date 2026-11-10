//! Gradients as option values.
//!
//! Chart.js only makes gradients through scriptable options (`ctx.createLinearGradient(…)`).
//! JSON can't call canvas methods, so KittyChart also accepts a gradient *description*
//! wherever a color goes, either directly in the config or returned by a scriptable function:
//!
//! ```json
//! { "type": "linear", "x0": "left", "y0": "bottom", "x1": "left", "y1": "top",
//!   "stops": [[0, "rgba(54,162,235,0)"], [1, "rgb(54,162,235)"]] }
//! { "type": "radial", "x0": "centerX", "y0": "centerY", "r0": 0,
//!   "x1": "centerX", "y1": "centerY", "r1": 120, "stops": [...] }
//! { "type": "conic", "angle": 0, "x": "centerX", "y": "centerY", "stops": [...] }
//! ```
//!
//! Coordinates are CSS pixels, or keywords for the chart area's `left`, `right`, `top`,
//! `bottom`, `centerX` and `centerY` (also written `chartArea.left` …). Angles are radians.

use kittychart_canvas::{ColorStop, Paint};
use serde_json::Value;

use crate::options::as_num;
use crate::scale::ChartArea;

/// A CSS color string or a gradient description, as a paint.
pub fn paint(v: &Value, area: &ChartArea) -> Option<Paint> {
    match v {
        Value::String(s) => crate::color::parse(s).map(Paint::Solid),
        Value::Object(o) => {
            let stops: Vec<ColorStop> = o
                .get("stops")?
                .as_array()?
                .iter()
                .filter_map(|s| {
                    let (offset, color) = match s {
                        Value::Array(a) => (a.first()?, a.get(1)?),
                        Value::Object(o) => (o.get("offset")?, o.get("color")?),
                        _ => return None,
                    };
                    Some(ColorStop {
                        offset: as_num(offset)?.clamp(0.0, 1.0) as f32,
                        color: crate::color::parse(color.as_str()?)?,
                    })
                })
                .collect();
            if stops.is_empty() {
                return None;
            }
            let c = |k: &str, default: f64| coord(o.get(k), area).unwrap_or(default) as f32;
            match o.get("type").and_then(Value::as_str)? {
                "linear" => Some(Paint::Linear {
                    x0: c("x0", area.left),
                    y0: c("y0", area.bottom),
                    x1: c("x1", area.left),
                    y1: c("y1", area.top),
                    stops,
                }),
                "radial" => {
                    let (cx, cy) = (area.left + area.width() / 2.0, area.top + area.height() / 2.0);
                    Some(Paint::Radial {
                        x0: c("x0", cx),
                        y0: c("y0", cy),
                        r0: c("r0", 0.0),
                        x1: c("x1", cx),
                        y1: c("y1", cy),
                        r1: c("r1", area.width().min(area.height()) / 2.0),
                        stops,
                    })
                }
                "conic" => Some(Paint::Conic {
                    start_angle: o.get("angle").and_then(as_num).unwrap_or(0.0) as f32,
                    x: c("x", area.left + area.width() / 2.0),
                    y: c("y", area.top + area.height() / 2.0),
                    stops,
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A coordinate: a number, or a chart-area keyword.
fn coord(v: Option<&Value>, a: &ChartArea) -> Option<f64> {
    match v? {
        Value::String(s) => Some(match s.strip_prefix("chartArea.").unwrap_or(s) {
            "left" => a.left,
            "right" => a.right,
            "top" => a.top,
            "bottom" => a.bottom,
            "centerX" => a.left + a.width() / 2.0,
            "centerY" => a.top + a.height() / 2.0,
            other => other.trim().parse().ok()?,
        }),
        other => as_num(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keywords_resolve_against_the_chart_area() {
        let area = ChartArea { left: 10.0, top: 20.0, right: 110.0, bottom: 220.0 };
        let p = paint(
            &json!({"type": "linear", "x0": 0, "y0": "chartArea.bottom", "x1": "centerX", "y1": "top",
                    "stops": [[0, "red"], {"offset": 1, "color": "blue"}]}),
            &area,
        )
        .unwrap();
        let Paint::Linear { x0, y0, x1, y1, stops } = p else { panic!() };
        assert_eq!((x0, y0, x1, y1), (0.0, 220.0, 60.0, 20.0));
        assert_eq!(stops.len(), 2);
        assert!(paint(&json!({"type": "linear", "stops": []}), &area).is_none());
        assert_eq!(paint(&json!("red"), &area), Some(Paint::Solid(crate::color::parse("red").unwrap())));
    }
}
