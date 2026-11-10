//! Drawing helpers shared by scales, plugins and elements (ports of `helpers.canvas`).

use kittychart_canvas::{Canvas, Color, Paint, TextAlign, TextBaseline};

use crate::config::Label;
use crate::options::FontSpec;

/// Options for [`render_text`] (Chart.js `RenderTextOpts`).
#[derive(Debug, Clone, Default)]
pub struct TextOpts {
    pub color: Option<Color>,
    pub translation: Option<(f64, f64)>,
    pub rotation: Option<f64>,
    pub align: Option<TextAlign>,
    pub baseline: Option<TextBaseline>,
    pub max_width: Option<f64>,
    pub stroke_color: Option<Color>,
    pub stroke_width: f64,
    pub strikethrough: bool,
    /// Filled rectangle behind the text, in the translated/rotated frame.
    pub backdrop: Option<(f64, f64, f64, f64, Color)>,
}

/// `renderText`: draw one or more lines starting at `(x, y)`, stepping by the font line height.
pub fn render_text(c: &mut dyn Canvas, label: &Label, x: f64, y: f64, font: &FontSpec, o: &TextOpts) {
    c.save();
    c.set_font(font.to_canvas());
    if let Some((tx, ty)) = o.translation {
        c.translate(tx as f32, ty as f32);
    }
    if let Some(r) = o.rotation {
        c.rotate(r as f32);
    }
    if let Some(col) = o.color {
        c.set_fill_style(Paint::Solid(col));
    }
    if let Some(a) = o.align {
        c.set_text_align(a);
    }
    if let Some(b) = o.baseline {
        c.set_text_baseline(b);
    }
    let stroke = o.stroke_width > 0.0 && o.stroke_color.is_some();
    let mut y = y;
    for line in label.lines() {
        if let Some((l, t, w, h, col)) = o.backdrop {
            c.save();
            c.set_fill_style(Paint::Solid(col));
            c.fill_rect(l as f32, t as f32, w as f32, h as f32);
            c.restore();
        }
        let max_w = o.max_width.map(|m| m as f32);
        if stroke {
            c.set_stroke_style(Paint::Solid(o.stroke_color.unwrap_or(Color::BLACK)));
            c.set_line_width(o.stroke_width as f32);
            c.stroke_text(line, x as f32, y as f32, max_w);
        }
        c.fill_text(line, x as f32, y as f32, max_w);
        if o.strikethrough {
            let m = c.measure_text(line);
            let left = x as f32 - m.actual_bounding_box_left;
            let right = x as f32 + m.actual_bounding_box_right;
            let top = y as f32 - m.actual_bounding_box_ascent;
            let bottom = y as f32 + m.actual_bounding_box_descent;
            let yd = (top + bottom) / 2.0;
            c.save();
            c.set_stroke_style(Paint::Solid(o.color.unwrap_or(Color::BLACK)));
            c.set_line_width(2.0);
            c.begin_path();
            c.move_to(left, yd);
            c.line_to(right, yd);
            c.stroke();
            c.restore();
        }
        y += font.line_height;
    }
    c.restore();
}

/// `addRoundedRectPath`.
pub fn rounded_rect_path(c: &mut dyn Canvas, x: f64, y: f64, w: f64, h: f64, r: crate::options::Corners) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let f = |v: f64| v as f32;
    c.arc(f(x + r.top_left), f(y + r.top_left), f(r.top_left), f(1.5 * PI), f(PI), true);
    c.line_to(f(x), f(y + h - r.bottom_left));
    c.arc(f(x + r.bottom_left), f(y + h - r.bottom_left), f(r.bottom_left), f(PI), f(FRAC_PI_2), true);
    c.line_to(f(x + w - r.bottom_right), f(y + h));
    c.arc(f(x + w - r.bottom_right), f(y + h - r.bottom_right), f(r.bottom_right), f(FRAC_PI_2), 0.0, true);
    c.line_to(f(x + w), f(y + r.top_right));
    c.arc(f(x + w - r.top_right), f(y + r.top_right), f(r.top_right), 0.0, f(-FRAC_PI_2), true);
    c.line_to(f(x + r.top_left), f(y));
}

/// Clip to an axis-aligned rectangle until the matching `restore`.
pub fn clip_area(c: &mut dyn Canvas, left: f64, top: f64, right: f64, bottom: f64) {
    c.save();
    c.begin_path();
    c.rect(left as f32, top as f32, (right - left) as f32, (bottom - top) as f32);
    c.clip();
    c.begin_path();
}

/// Chart.js `pointStyle` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PointStyle {
    #[default]
    Circle,
    Cross,
    CrossRot,
    Dash,
    Line,
    Rect,
    RectRounded,
    RectRot,
    Star,
    Triangle,
    /// `pointStyle: false`
    None,
}

impl PointStyle {
    pub fn from_value(v: Option<&serde_json::Value>) -> PointStyle {
        match v {
            Some(serde_json::Value::Bool(false)) => PointStyle::None,
            Some(serde_json::Value::String(s)) => match s.as_str() {
                "cross" => PointStyle::Cross,
                "crossRot" => PointStyle::CrossRot,
                "dash" => PointStyle::Dash,
                "line" => PointStyle::Line,
                "rect" => PointStyle::Rect,
                "rectRounded" => PointStyle::RectRounded,
                "rectRot" => PointStyle::RectRot,
                "star" => PointStyle::Star,
                "triangle" => PointStyle::Triangle,
                _ => PointStyle::Circle,
            },
            _ => PointStyle::Circle,
        }
    }
}

/// `drawPointLegend` (and `drawPoint` with `w = None`). Fill and stroke styles must be set.
#[allow(clippy::too_many_arguments)]
pub fn draw_point(
    c: &mut dyn Canvas,
    style: PointStyle,
    radius: f64,
    rotation_deg: f64,
    border_width: f64,
    x: f64,
    y: f64,
    w: Option<f64>,
) {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};
    if radius.is_nan() || radius <= 0.0 {
        return;
    }
    let f = |v: f64| v as f32;
    let mut rad = rotation_deg.to_radians();
    let two_thirds_pi = PI * 2.0 / 3.0;
    c.begin_path();
    match style {
        PointStyle::Circle => {
            match w {
                Some(w) if w > 0.0 => {
                    c.ellipse(f(x), f(y), f(w / 2.0), f(radius), 0.0, 0.0, f(2.0 * PI), false)
                }
                _ => c.arc(f(x), f(y), f(radius), 0.0, f(2.0 * PI), false),
            }
            c.close_path();
        }
        PointStyle::Triangle => {
            let width = w.map_or(radius, |w| w / 2.0);
            c.move_to(f(x + rad.sin() * width), f(y - rad.cos() * radius));
            rad += two_thirds_pi;
            c.line_to(f(x + rad.sin() * width), f(y - rad.cos() * radius));
            rad += two_thirds_pi;
            c.line_to(f(x + rad.sin() * width), f(y - rad.cos() * radius));
            c.close_path();
        }
        PointStyle::RectRounded => {
            let corner = radius * 0.516;
            let size = radius - corner;
            let x_off = (rad + FRAC_PI_4).cos() * size;
            let x_off_w = (rad + FRAC_PI_4).cos() * w.map_or(size, |w| w / 2.0 - corner);
            let y_off = (rad + FRAC_PI_4).sin() * size;
            let y_off_w = (rad + FRAC_PI_4).sin() * w.map_or(size, |w| w / 2.0 - corner);
            c.arc(f(x - x_off_w), f(y - y_off), f(corner), f(rad - PI), f(rad - FRAC_PI_2), false);
            c.arc(f(x + y_off_w), f(y - x_off), f(corner), f(rad - FRAC_PI_2), f(rad), false);
            c.arc(f(x + x_off_w), f(y + y_off), f(corner), f(rad), f(rad + FRAC_PI_2), false);
            c.arc(f(x - y_off_w), f(y + x_off), f(corner), f(rad + FRAC_PI_2), f(rad + PI), false);
            c.close_path();
        }
        PointStyle::Rect | PointStyle::RectRot => {
            if style == PointStyle::Rect && rotation_deg == 0.0 {
                let size = std::f64::consts::FRAC_1_SQRT_2 * radius;
                let width = w.map_or(size, |w| w / 2.0);
                c.rect(f(x - width), f(y - size), f(2.0 * width), f(2.0 * size));
            } else {
                if style == PointStyle::Rect {
                    rad += FRAC_PI_4;
                }
                let x_off_w = rad.cos() * w.map_or(radius, |w| w / 2.0);
                let x_off = rad.cos() * radius;
                let y_off = rad.sin() * radius;
                let y_off_w = rad.sin() * w.map_or(radius, |w| w / 2.0);
                c.move_to(f(x - x_off_w), f(y - y_off));
                c.line_to(f(x + y_off_w), f(y - x_off));
                c.line_to(f(x + x_off_w), f(y + y_off));
                c.line_to(f(x - y_off_w), f(y + x_off));
                c.close_path();
            }
        }
        PointStyle::Cross | PointStyle::CrossRot | PointStyle::Star => {
            if style == PointStyle::CrossRot {
                rad += FRAC_PI_4;
            }
            let cross = |rad: f64, c: &mut dyn Canvas| {
                let x_off_w = rad.cos() * w.map_or(radius, |w| w / 2.0);
                let x_off = rad.cos() * radius;
                let y_off = rad.sin() * radius;
                let y_off_w = rad.sin() * w.map_or(radius, |w| w / 2.0);
                c.move_to(f(x - x_off_w), f(y - y_off));
                c.line_to(f(x + x_off_w), f(y + y_off));
                c.move_to(f(x + y_off_w), f(y - x_off));
                c.line_to(f(x - y_off_w), f(y + x_off));
            };
            cross(rad, c);
            if style == PointStyle::Star {
                cross(rad + FRAC_PI_4, c);
            }
        }
        PointStyle::Line => {
            let x_off = w.map_or(rad.cos() * radius, |w| w / 2.0);
            let y_off = rad.sin() * radius;
            c.move_to(f(x - x_off), f(y - y_off));
            c.line_to(f(x + x_off), f(y + y_off));
        }
        PointStyle::Dash => {
            c.move_to(f(x), f(y));
            c.line_to(f(x + rad.cos() * w.map_or(radius, |w| w / 2.0)), f(y + rad.sin() * radius));
        }
        PointStyle::None => c.close_path(),
    }
    c.fill();
    if border_width > 0.0 {
        c.stroke();
    }
}
