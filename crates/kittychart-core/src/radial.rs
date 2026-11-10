//! `radialLinear` scale (radar and polar area charts): a port of Chart.js `scale.radialLinear.js`.
//!
//! It is a `chartArea` box: rather than taking part in layout, it is given the final chart area.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use kittychart_canvas::{Canvas, Paint, TextAlign, TextBaseline};
use serde_json::Value;

use crate::arc::normalize_angle;
use crate::config::Label;
use crate::draw::{TextOpts, render_text, rounded_rect_path};
use crate::format;
use crate::math::{js_round, sign, to_degrees, to_radians};
use crate::options::{Corners, FontSpec, Padding, Scopes, as_num, to_dimension};
use crate::scale::{ChartArea, TickGen, generate_ticks};

#[derive(Debug, Clone)]
pub struct PointLabelItem {
    pub visible: bool,
    pub x: f64,
    pub y: f64,
    pub align: TextAlign,
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[derive(Debug, Clone)]
pub struct RadialScale {
    pub id: String,
    config: Value,
    routes: Value,
    font_fallback: Value,
    labels: Vec<Label>,
    pub min: f64,
    pub max: f64,
    range: (f64, f64),
    user_min: Option<f64>,
    user_max: Option<f64>,
    suggested_min: Option<f64>,
    suggested_max: Option<f64>,
    pub ticks: Vec<(f64, String)>,
    pub point_labels: Vec<Label>,
    pub x_center: f64,
    pub y_center: f64,
    pub drawing_area: f64,
    point_label_items: Vec<PointLabelItem>,
}

impl RadialScale {
    pub fn new(
        id: &str,
        config: Value,
        routes: Value,
        font_fallback: Value,
        labels: Vec<Label>,
    ) -> RadialScale {
        let s = Scopes::new([&config]);
        let num = |k: &str| s.get(k).and_then(as_num).filter(|v| v.is_finite());
        RadialScale {
            id: id.to_owned(),
            user_min: num("min"),
            user_max: num("max"),
            suggested_min: num("suggestedMin"),
            suggested_max: num("suggestedMax"),
            config: config.clone(),
            routes,
            font_fallback,
            labels,
            min: 0.0,
            max: 0.0,
            range: (0.0, 0.0),
            ticks: vec![],
            point_labels: vec![],
            x_center: 0.0,
            y_center: 0.0,
            drawing_area: 0.0,
            point_label_items: vec![],
        }
    }

    fn opts(&self) -> Scopes<'_> {
        Scopes::new([&self.config])
    }
    fn ticks_opts(&self) -> Scopes<'_> {
        self.opts().child("ticks").then([&self.routes["ticks"]])
    }
    fn grid_opts(&self) -> Scopes<'_> {
        self.opts().child("grid").then([&self.routes["grid"]])
    }
    /// `angleLines` falls back to `grid` (descriptor `_fallback: 'grid'`), then its route.
    fn angle_lines_opts(&self) -> Scopes<'_> {
        let o = self.opts();
        let mut s = o.child("angleLines");
        s = s.then([&self.routes["angleLines"]]);
        s.then_scopes(&o.child("grid"))
    }
    fn point_label_opts(&self) -> Scopes<'_> {
        self.opts().child("pointLabels").then([&self.routes["pointLabels"]])
    }
    fn font_of(&self, s: &Scopes<'_>) -> FontSpec {
        FontSpec::resolve(&s.child("font"), &Scopes::new([&self.font_fallback]))
    }
    pub fn circular_grid(&self) -> bool {
        self.grid_opts().bool_or("circular", false)
    }

    /// `getBaseValue()` (linear scale base).
    pub fn base_value(&self) -> f64 {
        if self.min < 0.0 && self.max < 0.0 {
            self.max
        } else if self.min > 0.0 && self.max > 0.0 {
            self.min
        } else {
            0.0
        }
    }

    pub fn reverse(&self) -> bool {
        self.opts().bool_or("reverse", false)
    }
    fn displayed(&self) -> bool {
        self.opts().get("display").and_then(Value::as_bool) != Some(false)
    }

    // ------------------------------------------------------------------ data limits

    /// `determineDataLimits` (`getMinMax(false)`) + `handleTickRangeOptions` + `_addGrace`.
    pub fn determine_data_limits(&mut self, ranges: &[(f64, f64)]) {
        let min_defined = self.user_min.is_some();
        let max_defined = self.user_max.is_some();
        let mut min = self.user_min.or(self.suggested_min).unwrap_or(f64::INFINITY);
        let mut max = self.user_max.or(self.suggested_max).unwrap_or(f64::NEG_INFINITY);
        if !(min_defined && max_defined) {
            for &(a, b) in ranges {
                if !min_defined {
                    min = min.min(a);
                }
                if !max_defined {
                    max = max.max(b);
                }
            }
        }
        let fin = |v: f64, alt: f64| if v.is_finite() { v } else { alt };
        let (min, max) = (fin(min, fin(max, min)), fin(max, fin(min, max)));
        let (mut min, mut max) =
            (if min.is_finite() { min } else { 0.0 }, if max.is_finite() { max } else { 0.0 });
        let begin_at_zero = self.opts().bool_or("beginAtZero", false);
        if begin_at_zero {
            if sign(min) < 0.0 && sign(max) < 0.0 {
                if !max_defined {
                    max = 0.0;
                }
            } else if sign(min) > 0.0 && sign(max) > 0.0 && !min_defined {
                min = 0.0;
            }
        }
        if min == max {
            let off = if max == 0.0 { 1.0 } else { (max * 0.05).abs() };
            if !max_defined {
                max += off;
            }
            if !begin_at_zero && !min_defined {
                min -= off;
            }
        }
        self.min = min;
        self.max = max;
        let change = to_dimension(self.opts().get("grace"), (max - min) / 2.0);
        let keep = |v: f64, add: f64| if begin_at_zero && v == 0.0 { 0.0 } else { v + add };
        self.range = (keep(min, -change.abs()), keep(max, change));
    }

    fn tick_backdrop_height(&self) -> f64 {
        let t = self.ticks_opts();
        if t.bool_or("display", true) && self.displayed() {
            let pad = Padding::from_value(t.get("backdropPadding"));
            let size = t
                .child("font")
                .num("size")
                .unwrap_or_else(|| self.font_fallback["size"].as_f64().unwrap_or(12.0));
            size + pad.height()
        } else {
            0.0
        }
    }

    // ------------------------------------------------------------------ layout

    /// Assigned the chart area after layout (`chartArea` boxes), then updated.
    pub fn update(&mut self, area: &ChartArea, c: &mut dyn Canvas) {
        // setDimensions
        let pad = self.tick_backdrop_height() / 2.0;
        let w = area.width() - 2.0 * pad;
        let h = area.height() - 2.0 * pad;
        self.x_center = (area.left + w / 2.0 + pad).floor();
        self.y_center = (area.top + h / 2.0 + pad).floor();
        self.drawing_area = (w.min(h) / 2.0).floor();

        // buildTicks (LinearScaleBase with the radial tick limit)
        let t = self.ticks_opts();
        let step = t.num("stepSize").filter(|s| *s > 0.0);
        let mut max_ticks_limit = t.num("maxTicksLimit");
        let mut max_ticks = if let Some(step) = step {
            ((self.max / step).ceil() - (self.min / step).floor() + 1.0).min(1000.0)
        } else {
            max_ticks_limit = Some(max_ticks_limit.filter(|v| *v != 0.0).unwrap_or(11.0));
            let bh = self.tick_backdrop_height();
            if bh > 0.0 { (self.drawing_area / bh).ceil() } else { f64::INFINITY }
        };
        if let Some(l) = max_ticks_limit {
            max_ticks = max_ticks.min(l);
        }
        let tick_font = self.font_of(&t);
        let tg = TickGen {
            max_ticks: max_ticks.max(2.0),
            bounds_ticks: self.opts().str("bounds").unwrap_or("ticks") == "ticks",
            min: self.user_min,
            max: self.user_max,
            precision: t.num("precision"),
            step,
            count: t.num("count"),
            max_digits: h / tick_font.line_height,
            horizontal: false,
            min_rotation: t.num_or("minRotation", 0.0),
            include_bounds: t.get("includeBounds").and_then(Value::as_bool) != Some(false),
        };
        let values = generate_ticks(&tg, self.range.0, self.range.1);
        drop(t);
        if tg.bounds_ticks {
            for v in &values {
                self.min = self.min.min(*v);
                self.max = self.max.max(*v);
            }
        }
        let mut values = values;
        if self.reverse() {
            values.reverse();
        }
        self.ticks =
            values.iter().enumerate().map(|(i, v)| (*v, format::numeric_tick(*v, i, &values))).collect();
        self.point_labels = self.labels.clone();

        // fit
        if self.displayed() && self.point_label_opts().bool_or("display", true) {
            self.fit_with_point_labels(area, c);
        }
    }

    fn index_angle(&self, index: usize) -> f64 {
        let mult = TAU / (self.point_labels.len().max(1)) as f64;
        normalize_angle(index as f64 * mult + to_radians(self.opts().num_or("startAngle", 0.0)))
    }

    pub fn distance_for_value(&self, value: f64) -> f64 {
        if value.is_nan() {
            return f64::NAN;
        }
        let k = self.drawing_area / (self.max - self.min);
        if self.reverse() { (self.max - value) * k } else { (value - self.min) * k }
    }

    /// `getPointPosition` → (x, y, angle).
    pub fn point_position(&self, index: usize, distance: f64, additional: f64) -> (f64, f64, f64) {
        let angle = self.index_angle(index) - FRAC_PI_2 + additional;
        (angle.cos() * distance + self.x_center, angle.sin() * distance + self.y_center, angle)
    }

    pub fn point_for_value(&self, index: usize, value: f64) -> (f64, f64, f64) {
        self.point_position(index, self.distance_for_value(value), 0.0)
    }

    /// Start angle for polar-area arcs: `getIndexAngle(0) - π/2`.
    pub fn start_angle(&self) -> f64 {
        self.index_angle(0) - 0.5 * PI
    }

    fn fit_with_point_labels(&mut self, area: &ChartArea, c: &mut dyn Canvas) {
        let pad = self.tick_backdrop_height() / 2.0;
        let orig = (area.left + pad, area.right - pad, area.top + pad, area.bottom - pad);
        let mut lim = orig;
        let n = self.point_labels.len();
        let pl = self.point_label_opts();
        let additional = if pl.bool_or("centerPointLabels", false) { PI / n as f64 } else { 0.0 };
        let font = self.font_of(&pl);
        let padding = pl.num_or("padding", 5.0);
        let auto = pl.str("display") == Some("auto");
        drop(pl);
        let mut sizes = Vec::with_capacity(n);
        c.save();
        c.set_font(font.to_canvas());
        for i in 0..n {
            let label = &self.point_labels[i];
            let w = label.lines().iter().map(|l| f64::from(c.measure_text(l).width)).fold(0.0, f64::max);
            let h = label.line_count() as f64 * font.line_height;
            sizes.push((w, h));
            let (px, py, _) = self.point_position(i, self.drawing_area + padding, additional);
            let ang_rad = normalize_angle(self.index_angle(i) + additional);
            let angle = js_round(to_degrees(ang_rad));
            let hl = limits(angle, px, w, 0.0, 180.0);
            let vl = limits(angle, py, h, 90.0, 270.0);
            let (sin, cos) = (ang_rad.sin().abs(), ang_rad.cos().abs());
            if hl.0 < orig.0 {
                lim.0 = lim.0.min(orig.0 - (orig.0 - hl.0) / sin);
            } else if hl.1 > orig.1 {
                lim.1 = lim.1.max(orig.1 + (hl.1 - orig.1) / sin);
            }
            if vl.0 < orig.2 {
                lim.2 = lim.2.min(orig.2 - (orig.2 - vl.0) / cos);
            } else if vl.1 > orig.3 {
                lim.3 = lim.3.max(orig.3 + (vl.1 - orig.3) / cos);
            }
        }
        c.restore();
        // setCenterPoint
        let (l, r, t, b) = (orig.0 - lim.0, lim.1 - orig.1, orig.2 - lim.2, lim.3 - orig.3);
        self.x_center += ((l - r) / 2.0).floor();
        self.y_center += ((t - b) / 2.0).floor();
        self.drawing_area -= (self.drawing_area / 2.0).min(l.max(r).max(t).max(b));

        // buildPointLabelItems
        let extra = self.tick_backdrop_height() / 2.0;
        let mut last_visible: Option<(f64, f64, f64, f64)> = None;
        let mut items = Vec::with_capacity(n);
        for (i, (w, h)) in sizes.into_iter().enumerate() {
            let (x, y0, a) = self.point_position(i, self.drawing_area + extra + padding, additional);
            let angle = js_round(to_degrees(normalize_angle(a + FRAC_PI_2)));
            let y = if angle == 90.0 || angle == 270.0 {
                y0 - h / 2.0
            } else if angle > 270.0 || angle < 90.0 {
                y0 - h
            } else {
                y0
            };
            let align = if angle == 0.0 || angle == 180.0 {
                TextAlign::Center
            } else if angle < 180.0 {
                TextAlign::Left
            } else {
                TextAlign::Right
            };
            let left = match align {
                TextAlign::Right => x - w,
                TextAlign::Center => x - w / 2.0,
                _ => x,
            };
            let mut item =
                PointLabelItem { visible: true, x, y, align, left, top: y, right: left + w, bottom: y + h };
            if auto {
                item.visible = last_visible.is_none_or(|(al, at, ar, ab)| {
                    let inside =
                        |px: f64, py: f64| px > al - 0.5 && px < ar + 0.5 && py > at - 0.5 && py < ab + 0.5;
                    !(inside(item.left, item.top)
                        || inside(item.left, item.bottom)
                        || inside(item.right, item.top)
                        || inside(item.right, item.bottom))
                });
                if item.visible {
                    last_visible = Some((item.left, item.top, item.right, item.bottom));
                }
            }
            items.push(item);
        }
        self.point_label_items = items;
    }

    // ------------------------------------------------------------------ drawing

    fn path_radius_line(&self, c: &mut dyn Canvas, radius: f64, circular: bool) {
        let n = self.point_labels.len();
        if circular {
            c.arc(self.x_center as f32, self.y_center as f32, radius as f32, 0.0, TAU as f32, false);
        } else {
            let (x, y, _) = self.point_position(0, radius, 0.0);
            c.move_to(x as f32, y as f32);
            for i in 1..n {
                let (x, y, _) = self.point_position(i, radius, 0.0);
                c.line_to(x as f32, y as f32);
            }
        }
    }

    pub fn draw(&self, c: &mut dyn Canvas) {
        if !self.displayed() {
            return;
        }
        let grid = self.grid_opts();
        let circular = grid.bool_or("circular", false);
        let n = self.point_labels.len();
        // drawBackground
        if let Some(bg) = self.opts().color_at("backgroundColor", None) {
            c.save();
            c.begin_path();
            let end = if self.reverse() { self.min } else { self.max };
            self.path_radius_line(c, self.distance_for_value(end), circular);
            c.close_path();
            c.set_fill_style(Paint::Solid(bg));
            c.fill();
            c.restore();
        }
        // drawGrid: point labels, radius lines, angle lines
        let pl = self.point_label_opts();
        if pl.bool_or("display", true) {
            let font = self.font_of(&pl);
            for i in (0..n).rev() {
                let Some(item) = self.point_label_items.get(i) else { continue };
                if !item.visible {
                    continue;
                }
                if let Some(bg) = pl.color_at("backdropColor", Some(i)) {
                    let p = Padding::from_value(pl.get("backdropPadding"));
                    let (l, t) = (item.left - p.left, item.top - p.top);
                    let (w, h) = (item.right - item.left + p.width(), item.bottom - item.top + p.height());
                    c.save();
                    c.set_fill_style(Paint::Solid(bg));
                    let r = Corners::from_value(pl.get("borderRadius"));
                    if r.any() {
                        c.begin_path();
                        rounded_rect_path(c, l, t, w, h, r);
                        c.fill();
                    } else {
                        c.fill_rect(l as f32, t as f32, w as f32, h as f32);
                    }
                    c.restore();
                }
                render_text(
                    c,
                    &self.point_labels[i],
                    item.x,
                    item.y + font.line_height / 2.0,
                    &font,
                    &TextOpts {
                        color: pl.color_at("color", Some(i)),
                        align: Some(item.align),
                        baseline: Some(TextBaseline::Middle),
                        ..TextOpts::default()
                    },
                );
            }
        }
        if grid.bool_or("display", true) {
            let border = self.opts().child("border");
            let dash: Vec<f32> = border.num_list("dash").iter().map(|d| *d as f32).collect();
            for (index, (value, _)) in self.ticks.iter().enumerate() {
                if index != 0 || self.min < 0.0 {
                    let offset = self.distance_for_value(*value);
                    let color = grid.color_at("color", Some(index));
                    let width = grid.num_at("lineWidth", Some(index)).unwrap_or(1.0);
                    let Some(color) = color else { continue };
                    if (!circular && n == 0) || width == 0.0 || offset < 0.0 {
                        continue;
                    }
                    c.save();
                    c.set_stroke_style(Paint::Solid(color));
                    c.set_line_width(width as f32);
                    c.set_line_dash(&dash);
                    c.set_line_dash_offset(border.num_or("dashOffset", 0.0) as f32);
                    c.begin_path();
                    self.path_radius_line(c, offset, circular);
                    c.close_path();
                    c.stroke();
                    c.restore();
                }
            }
        }
        let al = self.angle_lines_opts();
        if al.bool_or("display", true) {
            c.save();
            for i in (0..n).rev() {
                let width = al.num_at("lineWidth", Some(i)).unwrap_or(1.0);
                let Some(color) = al.color_at("color", Some(i)) else { continue };
                if width == 0.0 {
                    continue;
                }
                c.set_line_width(width as f32);
                c.set_stroke_style(Paint::Solid(color));
                let dash: Vec<f32> = al.num_list("borderDash").iter().map(|d| *d as f32).collect();
                c.set_line_dash(&dash);
                c.set_line_dash_offset(al.num_or("borderDashOffset", 0.0) as f32);
                let offset = self.distance_for_value(if self.reverse() { self.min } else { self.max });
                let (x, y, _) = self.point_position(i, offset, 0.0);
                c.begin_path();
                c.move_to(self.x_center as f32, self.y_center as f32);
                c.line_to(x as f32, y as f32);
                c.stroke();
            }
            c.restore();
        }
        // drawLabels
        let t = self.ticks_opts();
        if !t.bool_or("display", true) {
            return;
        }
        c.save();
        c.translate(self.x_center as f32, self.y_center as f32);
        c.rotate(self.index_angle(0) as f32);
        c.set_text_align(TextAlign::Center);
        c.set_text_baseline(TextBaseline::Middle);
        let font = self.font_of(&t);
        for (index, (value, label)) in self.ticks.iter().enumerate() {
            if index == 0 && self.min >= 0.0 && !self.reverse() {
                continue;
            }
            let offset = self.distance_for_value(*value);
            if t.bool_or("showLabelBackdrop", true) {
                c.set_font(font.to_canvas());
                let w = f64::from(c.measure_text(label).width);
                let p = Padding::from_value(t.get("backdropPadding"));
                if let Some(bg) = t.color_at("backdropColor", Some(index)) {
                    c.set_fill_style(Paint::Solid(bg));
                    c.fill_rect(
                        (-w / 2.0 - p.left) as f32,
                        (-offset - font.size / 2.0 - p.top) as f32,
                        (w + p.width()) as f32,
                        (font.size + p.height()) as f32,
                    );
                }
            }
            render_text(
                c,
                &Label::Text(label.clone()),
                0.0,
                -offset,
                &font,
                &TextOpts {
                    color: t.color_at("color", Some(index)),
                    stroke_color: t.color_at("textStrokeColor", Some(index)),
                    stroke_width: t.num_or("textStrokeWidth", 0.0),
                    ..TextOpts::default()
                },
            );
        }
        c.restore();
    }
}

/// `determineLimits(angle, pos, size, min, max)` → (start, end).
fn limits(angle: f64, pos: f64, size: f64, min: f64, max: f64) -> (f64, f64) {
    if angle == min || angle == max {
        (pos - size / 2.0, pos + size / 2.0)
    } else if angle < min || angle > max {
        (pos - size, pos)
    } else {
        (pos, pos + size)
    }
}
