//! Chart elements: points, lines and bars (ports of Chart.js `element.point.ts`,
//! `element.line.js`, `element.bar.js`, `helpers.curve.ts` and `helpers.segment.js`).

use kittychart_canvas::{Canvas, Color, FillRule, LineCap, LineJoin, Paint};

use crate::draw::{PointStyle, draw_point, rounded_rect_path};
use crate::math::{limit_value, sign};
use crate::options::{Corners, Padding};
use crate::scale::ChartArea;

// ------------------------------------------------------------------ point

#[derive(Debug, Clone, PartialEq)]
pub struct PointOptions {
    pub radius: f64,
    pub hover_radius: f64,
    pub hit_radius: f64,
    pub point_style: PointStyle,
    pub rotation: f64,
    pub border_width: f64,
    pub background_color: Option<Color>,
    pub border_color: Option<Color>,
}

impl PointOptions {
    /// `PointElement.size()`: diameter including border, sized for hover.
    pub fn size(&self) -> f64 {
        let r = if self.radius != 0.0 { self.radius.max(self.hover_radius) } else { 0.0 };
        let bw = if r != 0.0 { self.border_width } else { 0.0 };
        (r + bw) * 2.0
    }
}

#[derive(Debug, Clone)]
pub struct Point {
    pub x: f64,
    pub y: f64,
    pub skip: bool,
    pub stop: bool,
    pub cp1x: f64,
    pub cp1y: f64,
    pub cp2x: f64,
    pub cp2y: f64,
    pub options: PointOptions,
}

impl Point {
    pub fn draw(&self, c: &mut dyn Canvas, area: &ChartArea) {
        let o = &self.options;
        if self.skip || o.radius < 0.1 || !in_area(self.x, self.y, area, o.size() / 2.0) {
            return;
        }
        c.set_stroke_style(Paint::Solid(o.border_color.unwrap_or(Color::TRANSPARENT)));
        c.set_line_width(o.border_width as f32);
        c.set_fill_style(Paint::Solid(o.background_color.unwrap_or(Color::TRANSPARENT)));
        draw_point(c, o.point_style, o.radius, o.rotation, o.border_width, self.x, self.y, None);
    }
}

/// `_isPointInArea`.
pub fn in_area(x: f64, y: f64, area: &ChartArea, margin: f64) -> bool {
    let m = if margin == 0.0 { 0.5 } else { margin };
    x > area.left - m && x < area.right + m && y > area.top - m && y < area.bottom + m
}

// ------------------------------------------------------------------ line

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stepped {
    #[default]
    No,
    Before,
    After,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SpanGaps {
    No,
    Yes,
    /// Bridge gaps up to this distance on the index axis.
    Max(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LineOptions {
    pub border_width: f64,
    pub border_color: Option<Color>,
    pub background_color: Option<Color>,
    pub cap: LineCap,
    pub join: LineJoin,
    pub dash: Vec<f64>,
    pub dash_offset: f64,
    pub tension: f64,
    pub monotone: bool,
    pub stepped: Stepped,
    pub span_gaps: SpanGaps,
    pub cap_bezier_points: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub looped: bool,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub options: LineOptions,
}

impl Line {
    fn uses_curve(&self) -> bool {
        (self.options.tension != 0.0 || self.options.monotone) && self.options.stepped == Stepped::No
    }

    /// `updateControlPoints` → `_updateBezierControlPoints`.
    pub fn update_control_points(&self, points: &mut [Point], area: &ChartArea, index_axis_x: bool) {
        if !self.uses_curve() {
            return;
        }
        let o = &self.options;
        let idx: Vec<usize> = if o.span_gaps != SpanGaps::No {
            (0..points.len()).filter(|&i| !points[i].skip).collect()
        } else {
            (0..points.len()).collect()
        };
        if o.monotone {
            spline_curve_monotone(points, &idx, index_axis_x);
        } else {
            let n = idx.len();
            if n == 0 {
                return;
            }
            let mut prev = idx[0];
            for k in 0..n {
                let cur = idx[k];
                let next = idx[(k + 1).min(n - 1) % n];
                let (p, nx) = spline_curve(&points[prev], &points[cur], &points[next], o.tension);
                let pt = &mut points[cur];
                pt.cp1x = p.0;
                pt.cp1y = p.1;
                pt.cp2x = nx.0;
                pt.cp2y = nx.1;
                prev = cur;
            }
        }
        if o.cap_bezier_points {
            cap_bezier_points(points, &idx, area);
        }
    }

    /// `_computeSegments` (no loops for cartesian charts; no segment styling yet).
    #[allow(clippy::needless_range_loop)] // index-based, mirroring helpers.segment.js
    pub fn segments(&self, points: &[Point]) -> Vec<Segment> {
        let count = points.len();
        if count == 0 {
            return vec![];
        }
        let mut start = 0;
        while start < count && points[start].skip {
            start += 1;
        }
        start %= count;
        let mut end = count - 1;
        while end > start && points[end].skip {
            end -= 1;
        }
        if self.options.span_gaps == SpanGaps::Yes {
            return vec![Segment { start, end, looped: false }];
        }
        // solidSegments
        let mut result = Vec::new();
        let mut seg_start: Option<usize> = Some(start);
        let mut last: Option<usize> = Some(start);
        let mut prev_skip = points[start].skip;
        for e in start + 1..=end {
            let cur = &points[e];
            if cur.skip || cur.stop {
                if !prev_skip {
                    if let Some(s) = seg_start {
                        result.push(Segment { start: s, end: e - 1, looped: false });
                    }
                    seg_start = if cur.stop { Some(e) } else { None };
                    last = seg_start;
                }
            } else {
                last = Some(e);
                if prev_skip {
                    seg_start = Some(e);
                }
            }
            prev_skip = cur.skip;
        }
        if let (Some(s), Some(l)) = (seg_start, last) {
            result.push(Segment { start: s, end: l, looped: false });
        }
        result
    }

    /// Append the line's path for `seg` (`pathSegment`). Returns false if nothing was drawn.
    pub fn path_segment(
        &self,
        c: &mut dyn Canvas,
        points: &[Point],
        seg: Segment,
        reverse: bool,
        mut move_first: bool,
    ) {
        let ilen = seg.end.saturating_sub(seg.start);
        let mut prev: Option<&Point> = None;
        for i in 0..=ilen {
            let idx = seg.start + if reverse { ilen - i } else { i };
            let p = &points[idx];
            if p.skip {
                continue;
            }
            if move_first {
                c.move_to(p.x as f32, p.y as f32);
                move_first = false;
            } else {
                self.line_method(c, prev, p, reverse);
            }
            prev = Some(p);
        }
    }

    fn line_method(&self, c: &mut dyn Canvas, prev: Option<&Point>, t: &Point, flip: bool) {
        let Some(p) = prev else {
            c.line_to(t.x as f32, t.y as f32);
            return;
        };
        let f = |v: f64| v as f32;
        match self.options.stepped {
            Stepped::No if self.uses_curve() => {
                c.bezier_curve_to(
                    f(if flip { p.cp1x } else { p.cp2x }),
                    f(if flip { p.cp1y } else { p.cp2y }),
                    f(if flip { t.cp2x } else { t.cp1x }),
                    f(if flip { t.cp2y } else { t.cp1y }),
                    f(t.x),
                    f(t.y),
                );
            }
            Stepped::No => c.line_to(f(t.x), f(t.y)),
            Stepped::Middle => {
                let mid = (p.x + t.x) / 2.0;
                c.line_to(f(mid), f(p.y));
                c.line_to(f(mid), f(t.y));
                c.line_to(f(t.x), f(t.y));
            }
            mode => {
                if (mode == Stepped::After) != flip {
                    c.line_to(f(p.x), f(t.y));
                } else {
                    c.line_to(f(t.x), f(p.y));
                }
                c.line_to(f(t.x), f(t.y));
            }
        }
    }

    pub fn draw(&self, c: &mut dyn Canvas, points: &[Point]) {
        let o = &self.options;
        if points.is_empty() || o.border_width == 0.0 {
            return;
        }
        c.save();
        c.set_line_cap(o.cap);
        let dash: Vec<f32> = o.dash.iter().map(|d| *d as f32).collect();
        c.set_line_dash(&dash);
        c.set_line_dash_offset(o.dash_offset as f32);
        c.set_line_join(o.join);
        c.set_line_width(o.border_width as f32);
        c.set_stroke_style(Paint::Solid(o.border_color.unwrap_or(Color::TRANSPARENT)));
        // Chart.js strokes one Path2D holding every segment.
        c.begin_path();
        for seg in self.segments(points) {
            self.path_segment(c, points, seg, false, true);
        }
        c.stroke();
        c.restore();
    }
}

fn spline_curve(first: &Point, middle: &Point, after: &Point, t: f64) -> ((f64, f64), (f64, f64)) {
    let previous = if first.skip { middle } else { first };
    let next = if after.skip { middle } else { after };
    let d01 = ((middle.x - previous.x).powi(2) + (middle.y - previous.y).powi(2)).sqrt();
    let d12 = ((next.x - middle.x).powi(2) + (next.y - middle.y).powi(2)).sqrt();
    let mut s01 = d01 / (d01 + d12);
    let mut s12 = d12 / (d01 + d12);
    if s01.is_nan() {
        s01 = 0.0;
    }
    if s12.is_nan() {
        s12 = 0.0;
    }
    let fa = t * s01;
    let fb = t * s12;
    (
        (middle.x - fa * (next.x - previous.x), middle.y - fa * (next.y - previous.y)),
        (middle.x + fb * (next.x - previous.x), middle.y + fb * (next.y - previous.y)),
    )
}

#[allow(clippy::needless_range_loop)] // index-based, mirroring helpers.curve.ts
fn spline_curve_monotone(points: &mut [Point], idx: &[usize], index_x: bool) {
    let n = idx.len();
    let get = |pts: &[Point], k: usize| -> Option<usize> { (k < n && !pts[idx[k]].skip).then(|| idx[k]) };
    let ia = |p: &Point| if index_x { p.x } else { p.y };
    let va = |p: &Point| if index_x { p.y } else { p.x };
    let mut delta_k = vec![0.0; n];
    let mut m_k = vec![0.0; n];
    let mut current: Option<usize> = None;
    let mut after = get(points, 0);
    for i in 0..n {
        let before = current;
        current = after;
        after = get(points, i + 1);
        let Some(cur) = current else { continue };
        if let Some(a) = after {
            let slope = ia(&points[a]) - ia(&points[cur]);
            delta_k[i] = if slope != 0.0 { (va(&points[a]) - va(&points[cur])) / slope } else { 0.0 };
        }
        m_k[i] = if before.is_none() {
            delta_k[i]
        } else if after.is_none() {
            delta_k[i - 1]
        } else if sign(delta_k[i - 1]) != sign(delta_k[i]) {
            0.0
        } else {
            (delta_k[i - 1] + delta_k[i]) / 2.0
        };
    }
    // monotoneAdjust
    let mut after = get(points, 0);
    for i in 0..n.saturating_sub(1) {
        let current = after;
        after = get(points, i + 1);
        if current.is_none() || after.is_none() {
            continue;
        }
        if delta_k[i].abs() < f64::EPSILON {
            m_k[i] = 0.0;
            m_k[i + 1] = 0.0;
            continue;
        }
        let alpha = m_k[i] / delta_k[i];
        let beta = m_k[i + 1] / delta_k[i];
        let sq = alpha * alpha + beta * beta;
        if sq <= 9.0 {
            continue;
        }
        let tau = 3.0 / sq.sqrt();
        m_k[i] = alpha * tau * delta_k[i];
        m_k[i + 1] = beta * tau * delta_k[i];
    }
    // monotoneCompute
    let mut current: Option<usize> = None;
    let mut after = get(points, 0);
    for i in 0..n {
        let before = current;
        current = after;
        after = get(points, i + 1);
        let Some(cur) = current else { continue };
        let (ip, vp) = (ia(&points[cur]), va(&points[cur]));
        if let Some(b) = before {
            let delta = (ip - ia(&points[b])) / 3.0;
            let (ci, cv) = (ip - delta, vp - delta * m_k[i]);
            let p = &mut points[cur];
            if index_x {
                p.cp1x = ci;
                p.cp1y = cv;
            } else {
                p.cp1y = ci;
                p.cp1x = cv;
            }
        }
        if let Some(a) = after {
            let delta = (ia(&points[a]) - ip) / 3.0;
            let (ci, cv) = (ip + delta, vp + delta * m_k[i]);
            let p = &mut points[cur];
            if index_x {
                p.cp2x = ci;
                p.cp2y = cv;
            } else {
                p.cp2y = ci;
                p.cp2x = cv;
            }
        }
    }
}

fn cap_bezier_points(points: &mut [Point], idx: &[usize], area: &ChartArea) {
    let n = idx.len();
    if n == 0 {
        return;
    }
    let inside = |p: &Point| in_area(p.x, p.y, area, 0.0);
    let mut in_area_prev;
    let mut in_area_cur = false;
    let mut in_area_next = inside(&points[idx[0]]);
    for k in 0..n {
        in_area_prev = in_area_cur;
        in_area_cur = in_area_next;
        in_area_next = k < n - 1 && inside(&points[idx[k + 1]]);
        if !in_area_cur {
            continue;
        }
        let p = &mut points[idx[k]];
        if in_area_prev {
            p.cp1x = limit_value(p.cp1x, area.left, area.right);
            p.cp1y = limit_value(p.cp1y, area.top, area.bottom);
        }
        if in_area_next {
            p.cp2x = limit_value(p.cp2x, area.left, area.right);
            p.cp2y = limit_value(p.cp2y, area.top, area.bottom);
        }
    }
}

// ------------------------------------------------------------------ bar

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
    pub left: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BarOptions {
    pub background_color: Option<Color>,
    pub border_color: Option<Color>,
    pub border_width: Padding,
    pub border_radius: Corners,
    /// The radius option was an object (enables radius on all corners Chart.js-style).
    pub border_radius_is_object: bool,
    /// `borderSkipped`: `None` = false, `Some("start" | "end" | "middle" | "bottom" | … | "true")`.
    pub border_skipped: Option<String>,
    /// `inflateAmount`: `None` = 'auto'.
    pub inflate_amount: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Bar {
    pub x: f64,
    pub y: f64,
    pub base: f64,
    pub width: f64,
    pub height: f64,
    pub horizontal: bool,
    pub border_skipped: Edges,
    pub enable_border_radius: bool,
    pub inflate_amount: f64,
    pub options: BarOptions,
}

#[derive(Debug, Clone, Copy)]
struct RRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    radius: Corners,
}

impl Bar {
    fn bounds(&self) -> (f64, f64, f64, f64) {
        if self.horizontal {
            let half = self.height / 2.0;
            (self.x.min(self.base), self.y - half, self.x.max(self.base), self.y + half)
        } else {
            let half = self.width / 2.0;
            (self.x - half, self.y.min(self.base), self.x + half, self.y.max(self.base))
        }
    }

    fn bounding_rects(&self) -> (RRect, RRect) {
        let (left, top, right, bottom) = self.bounds();
        let width = right - left;
        let height = bottom - top;
        let (max_w, max_h) = (width / 2.0, height / 2.0);
        let s = self.border_skipped;
        let bw = self.options.border_width;
        let lim = |skip: bool, v: f64, max: f64| if skip { 0.0 } else { limit_value(v, 0.0, max) };
        let (t, r, b, l) = (
            lim(s.top, bw.top, max_h),
            lim(s.right, bw.right, max_w),
            lim(s.bottom, bw.bottom, max_h),
            lim(s.left, bw.left, max_w),
        );
        let br = self.options.border_radius;
        let max_r = max_w.min(max_h);
        let enable = self.enable_border_radius || self.options.border_radius_is_object;
        let radius = Corners {
            top_left: lim(!enable || s.top || s.left, br.top_left, max_r),
            top_right: lim(!enable || s.top || s.right, br.top_right, max_r),
            bottom_left: lim(!enable || s.bottom || s.left, br.bottom_left, max_r),
            bottom_right: lim(!enable || s.bottom || s.right, br.bottom_right, max_r),
        };
        let outer = RRect { x: left, y: top, w: width, h: height, radius };
        let inner = RRect {
            x: left + l,
            y: top + t,
            w: width - l - r,
            h: height - t - b,
            radius: Corners {
                top_left: (radius.top_left - t.max(l)).max(0.0),
                top_right: (radius.top_right - t.max(r)).max(0.0),
                bottom_left: (radius.bottom_left - b.max(l)).max(0.0),
                bottom_right: (radius.bottom_right - b.max(r)).max(0.0),
            },
        };
        (outer, inner)
    }

    pub fn draw(&self, c: &mut dyn Canvas) {
        let (outer, inner) = self.bounding_rects();
        let rounded = outer.radius.any();
        let add = |c: &mut dyn Canvas, r: RRect| {
            if rounded {
                rounded_rect_path(c, r.x, r.y, r.w, r.h, r.radius);
            } else {
                c.rect(r.x as f32, r.y as f32, r.w as f32, r.h as f32);
            }
        };
        c.save();
        if outer.w != inner.w || outer.h != inner.h {
            c.begin_path();
            add(c, inflate_rect(outer, self.inflate_amount, Some(inner)));
            c.clip();
            add(c, inflate_rect(inner, -self.inflate_amount, Some(outer)));
            c.set_fill_style(Paint::Solid(self.options.border_color.unwrap_or(Color::TRANSPARENT)));
            c.fill_with_rule(FillRule::EvenOdd);
        }
        c.begin_path();
        add(c, inflate_rect(inner, self.inflate_amount, None));
        c.set_fill_style(Paint::Solid(self.options.background_color.unwrap_or(Color::TRANSPARENT)));
        c.fill();
        c.restore();
    }
}

fn inflate_rect(r: RRect, amount: f64, reference: Option<RRect>) -> RRect {
    let differs = |a: f64, b: Option<f64>| b.is_none_or(|b| a != b);
    let x = if differs(r.x, reference.map(|f| f.x)) { -amount } else { 0.0 };
    let y = if differs(r.y, reference.map(|f| f.y)) { -amount } else { 0.0 };
    let w = (if differs(r.x + r.w, reference.map(|f| f.x + f.w)) { amount } else { 0.0 }) - x;
    let h = (if differs(r.y + r.h, reference.map(|f| f.y + f.h)) { amount } else { 0.0 }) - y;
    RRect { x: r.x + x, y: r.y + y, w: r.w + w, h: r.h + h, radius: r.radius }
}
