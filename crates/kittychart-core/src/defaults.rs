//! Chart.js 4.6 defaults (`Chart.defaults`) and per-type `overrides`, for supported features.
//!
//! These are mirrored from Chart.js's source: `core.defaults.js`, `core.scale.defaults.js`,
//! `core.layouts.defaults.js`, element/controller `defaults`, and plugin `defaults`. Option
//! "routes" (such as `scale.ticks.color → color`) are applied by the resolvers that read them.

use std::sync::LazyLock;

use serde_json::{Value, json};

pub static DEFAULTS: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "backgroundColor": "rgba(0,0,0,0.1)",
        "borderColor": "rgba(0,0,0,0.1)",
        "color": "#666",
        "font": {
            "family": "'Helvetica Neue', 'Helvetica', 'Arial', sans-serif",
            "size": 12,
            "style": "normal",
            "lineHeight": 1.2,
            "weight": null
        },
        "indexAxis": "x",
        "maintainAspectRatio": true,
        "aspectRatio": 2,
        "showLine": true,
        "layout": { "autoPadding": true, "padding": { "top": 0, "right": 0, "bottom": 0, "left": 0 } },
        "elements": {
            "line": {
                "borderCapStyle": "butt",
                "borderDash": [],
                "borderDashOffset": 0,
                "borderJoinStyle": "miter",
                "borderWidth": 3,
                "capBezierPoints": true,
                "cubicInterpolationMode": "default",
                "fill": false,
                "spanGaps": false,
                "stepped": false,
                "tension": 0
            },
            "point": {
                "borderWidth": 1,
                "hitRadius": 1,
                "hoverBorderWidth": 1,
                "hoverRadius": 4,
                "pointStyle": "circle",
                "radius": 3,
                "rotation": 0
            },
            "arc": {
                "borderAlign": "center",
                "borderColor": "#fff",
                "borderDash": [],
                "borderDashOffset": 0,
                "borderRadius": 0,
                "borderWidth": 2,
                "offset": 0,
                "spacing": 0,
                "spacingMode": "angular",
                "circular": true,
                "selfJoin": false
            },
            "bar": {
                "borderSkipped": "start",
                "borderWidth": 0,
                "borderRadius": 0,
                "inflateAmount": "auto"
            }
        },
        "datasets": {
            "line": { "showLine": true, "spanGaps": false },
            "bar": { "categoryPercentage": 0.8, "barPercentage": 0.9, "grouped": true },
            "scatter": { "showLine": false, "fill": false },
            "doughnut": {
                "cutout": "50%", "rotation": 0, "circumference": 360, "radius": "100%",
                "spacing": 0, "spacingMode": "angular", "indexAxis": "r"
            },
            "pie": {
                "cutout": 0, "rotation": 0, "circumference": 360, "radius": "100%",
                "spacing": 0, "spacingMode": "angular", "indexAxis": "r"
            },
            "bubble": {},
            "radar": { "indexAxis": "r", "showLine": true, "elements": { "line": { "fill": "start" } } },
            "polarArea": { "indexAxis": "r", "startAngle": 0, "spacing": 0, "spacingMode": "proportional" }
        },
        "plugins": {
            "colors": { "enabled": true, "forceOverride": false },
            "filler": { "propagate": true, "drawTime": "beforeDatasetDraw" },
            "title": {
                "align": "center",
                "display": false,
                "font": { "weight": "bold" },
                "fullSize": true,
                "padding": 10,
                "position": "top",
                "text": "",
                "weight": 2000
            },
            "subtitle": {
                "align": "center",
                "display": false,
                "font": { "weight": "normal" },
                "fullSize": true,
                "padding": 0,
                "position": "top",
                "text": "",
                "weight": 1500
            },
            "legend": {
                "display": true,
                "position": "top",
                "align": "center",
                "fullSize": true,
                "reverse": false,
                "weight": 1000,
                "labels": { "boxWidth": 40, "padding": 10 },
                "title": { "display": false, "position": "center", "text": "" }
            }
        },
        "scale": {
            "display": true,
            "offset": false,
            "reverse": false,
            "beginAtZero": false,
            "bounds": "ticks",
            "clip": true,
            "grace": 0,
            "grid": {
                "display": true,
                "lineWidth": 1,
                "drawOnChartArea": true,
                "drawTicks": true,
                "tickLength": 8,
                "offset": false
            },
            "border": { "display": true, "dash": [], "dashOffset": 0.0, "width": 1 },
            "title": { "display": false, "text": "", "padding": { "top": 4, "bottom": 4 } },
            "ticks": {
                "minRotation": 0,
                "maxRotation": 50,
                "mirror": false,
                "textStrokeWidth": 0,
                "textStrokeColor": "",
                "padding": 3,
                "display": true,
                "autoSkip": true,
                "autoSkipPadding": 3,
                "labelOffset": 0,
                "minor": {},
                "major": {},
                "align": "center",
                "crossAlign": "near",
                "showLabelBackdrop": false,
                "backdropColor": "rgba(255, 255, 255, 0.75)",
                "backdropPadding": 2
            }
        },
        "scales": {
            "category": {},
            "linear": {},
            "logarithmic": { "ticks": { "major": { "enabled": true } } },
            "time": {
                "bounds": "data",
                "time": { "parser": false, "unit": false, "round": false, "isoWeekday": false, "minUnit": "millisecond", "displayFormats": {} },
                "ticks": { "source": "auto", "major": { "enabled": false } }
            },
            "timeseries": {
                "bounds": "data",
                "time": { "parser": false, "unit": false, "round": false, "isoWeekday": false, "minUnit": "millisecond", "displayFormats": {} },
                "ticks": { "source": "auto", "major": { "enabled": false } }
            },
            "radialLinear": {
                "display": true,
                "animate": true,
                "position": "chartArea",
                "angleLines": { "display": true, "lineWidth": 1, "borderDash": [], "borderDashOffset": 0.0 },
                "grid": { "circular": false },
                "startAngle": 0,
                "ticks": { "showLabelBackdrop": true },
                "pointLabels": {
                    "backdropPadding": 2,
                    "display": true,
                    "font": { "size": 10 },
                    "padding": 5,
                    "centerPointLabels": false
                }
            }
        }
    })
});

/// Per chart type `overrides` (controller `static overrides`).
pub static OVERRIDES: LazyLock<Value> = LazyLock::new(|| {
    json!({
        "line": {
            "scales": {
                "_index_": { "type": "category" },
                "_value_": { "type": "linear" }
            }
        },
        "bar": {
            "scales": {
                "_index_": { "type": "category", "offset": true, "grid": { "offset": true } },
                "_value_": { "type": "linear", "beginAtZero": true }
            }
        },
        "scatter": {
            "interaction": { "mode": "point" },
            "scales": { "x": { "type": "linear" }, "y": { "type": "linear" } }
        },
        "bubble": {
            "scales": { "x": { "type": "linear" }, "y": { "type": "linear" } }
        },
        "radar": { "aspectRatio": 1, "scales": { "r": { "type": "radialLinear" } } },
        "polarArea": {
            "aspectRatio": 1,
            "scales": {
                "r": {
                    "type": "radialLinear",
                    "angleLines": { "display": false },
                    "beginAtZero": true,
                    "grid": { "circular": true },
                    "pointLabels": { "display": false },
                    "startAngle": 0
                }
            }
        },
        "doughnut": { "aspectRatio": 1 },
        "pie": { "aspectRatio": 1 }
    })
});

pub fn overrides_for(chart_type: &str) -> &'static Value {
    static EMPTY: LazyLock<Value> = LazyLock::new(|| json!({}));
    OVERRIDES.get(chart_type).unwrap_or(&EMPTY)
}

/// Chart.js `colors` plugin palette.
pub const BORDER_COLORS: [&str; 7] = [
    "rgb(54, 162, 235)",
    "rgb(255, 99, 132)",
    "rgb(255, 159, 64)",
    "rgb(255, 205, 86)",
    "rgb(75, 192, 192)",
    "rgb(153, 102, 255)",
    "rgb(201, 203, 207)",
];
pub const BACKGROUND_COLORS: [&str; 7] = [
    "rgba(54, 162, 235, 0.5)",
    "rgba(255, 99, 132, 0.5)",
    "rgba(255, 159, 64, 0.5)",
    "rgba(255, 205, 86, 0.5)",
    "rgba(75, 192, 192, 0.5)",
    "rgba(153, 102, 255, 0.5)",
    "rgba(201, 203, 207, 0.5)",
];
