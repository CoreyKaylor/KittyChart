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
    pub background_color: Option<Paint>,
    pub border_color: Option<Paint>,
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
    /// Angle on a radial scale (radar), for fills that run along the angle.
    pub angle: f64,
    pub options: PointOptions,
}

impl Point {
    pub fn draw(&self, c: &mut dyn Canvas, area: &ChartArea) {
        let o = &self.options;
        if self.skip || o.radius < 0.1 || !in_area(self.x, self.y, area, o.size() / 2.0) {
            return;
        }
        c.set_stroke_style(o.border_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
        c.set_line_width(o.border_width as f32);
        c.set_fill_style(o.background_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
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
    pub border_color: Option<Paint>,
    pub background_color: Option<Paint>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start: usize,
    pub end: usize,
    pub looped: bool,
    /// Index into [`Line::styles`] when segment styling split the line here.
    pub style: Option<usize>,
}

/// One segment's style (`readStyle`): unset fields fall back to the line's options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SegStyle {
    pub background_color: Option<Paint>,
    pub border_color: Option<Paint>,
    pub border_width: Option<f64>,
    pub dash: Option<Vec<f64>>,
    pub dash_offset: Option<f64>,
    pub cap: Option<LineCap>,
    pub join: Option<LineJoin>,
}

/// The point property a fill or interpolation runs along.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prop {
    X,
    Y,
    Angle,
}

impl Point {
    pub fn get(&self, p: Prop) -> f64 {
        match p {
            Prop::X => self.x,
            Prop::Y => self.y,
            Prop::Angle => self.angle,
        }
    }

    /// A plain point (for boundary lines built by the filler).
    pub fn at(x: f64, y: f64, angle: f64) -> Point {
        Point {
            x,
            y,
            skip: false,
            stop: false,
            cp1x: x,
            cp1y: y,
            cp2x: x,
            cp2y: y,
            angle,
            options: PointOptions {
                radius: 0.0,
                hover_radius: 0.0,
                hit_radius: 0.0,
                point_style: PointStyle::Circle,
                rotation: 0.0,
                border_width: 0.0,
                background_color: None,
                border_color: None,
            },
        }
    }
}

impl LineOptions {
    /// Straight, unstyled line options (filler boundary and stack lines: `{tension: 0}`).
    pub fn plain() -> LineOptions {
        LineOptions {
            border_width: 0.0,
            border_color: None,
            background_color: None,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            dash: vec![],
            dash_offset: 0.0,
            tension: 0.0,
            monotone: false,
            stepped: Stepped::No,
            span_gaps: SpanGaps::No,
            cap_bezier_points: false,
        }
    }
}

/// A line element: options plus its points (Chart.js keeps them on the element).
#[derive(Debug, Clone)]
pub struct Line {
    pub options: LineOptions,
    /// `_loop`: the line closes back on itself (radar).
    pub looped: bool,
    /// `_fullLoop`: every label has a point.
    pub full_loop: bool,
    /// With `segment` options: the segments split by style (see [`Line::split_by_styles`]).
    pub styled: Option<Vec<Segment>>,
    pub styles: Vec<SegStyle>,
}

impl Line {
    pub fn new(options: LineOptions) -> Line {
        Line { options, looped: false, full_loop: false, styled: None, styles: Vec::new() }
    }

    fn uses_curve(&self) -> bool {
        (self.options.tension != 0.0 || self.options.monotone) && self.options.stepped == Stepped::No
    }

    /// `updateControlPoints` → `_updateBezierControlPoints`.
    pub fn update_control_points(&self, points: &mut [Point], area: &ChartArea, index_axis_x: bool) {
        if !self.uses_curve() {
            return;
        }
        let o = &self.options;
        let looped = if o.span_gaps != SpanGaps::No { self.looped } else { self.full_loop };
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
            let mut prev = if looped { idx[n - 1] } else { idx[0] };
            for k in 0..n {
                let cur = idx[k];
                let next = idx[(k + 1).min(n - usize::from(!looped)) % n];
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

    /// `_computeSegments`: the drawable segments, split by style when `segment` options apply.
    pub fn segments(&self, points: &[Point]) -> Vec<Segment> {
        if let Some(s) = &self.styled {
            return s.clone();
        }
        self.solid_segments(points)
    }

    /// `splitByStyles`: split the solid segments wherever the style that `style(p0, p1)`
    /// computes for consecutive points changes.
    pub fn split_by_styles(&mut self, points: &[Point], mut style: impl FnMut(usize, usize) -> SegStyle) {
        let segments = self.solid_segments(points);
        let count = points.len();
        if count == 0 || segments.is_empty() {
            return;
        }
        let o = &self.options;
        let base = SegStyle {
            background_color: o.background_color.clone(),
            border_color: o.border_color.clone(),
            border_width: Some(o.border_width),
            dash: Some(o.dash.clone()),
            dash_offset: Some(o.dash_offset),
            cap: Some(o.cap),
            join: Some(o.join),
        };
        let span_gaps = o.span_gaps != SpanGaps::No;
        let mut result: Vec<Segment> = Vec::new();
        let mut styles: Vec<SegStyle> = Vec::new();
        let mut prev_style = base;
        let mut start = segments[0].start;
        // addStyle
        let mut add = |s: usize,
                       e: usize,
                       looped: bool,
                       st: &SegStyle,
                       start: &mut usize,
                       prev_style: &mut SegStyle| {
            if s == e {
                return;
            }
            let (mut s, mut e) = (s + count, e);
            if span_gaps {
                while points[s % count].skip {
                    s += 1;
                }
                while points[e % count].skip {
                    e -= 1;
                }
            } else {
                while points[s % count].skip {
                    s -= 1;
                }
                while points[e % count].skip {
                    e += 1;
                }
            }
            if s % count != e % count {
                styles.push(st.clone());
                result.push(Segment {
                    start: s % count,
                    end: e % count,
                    looped,
                    style: Some(styles.len() - 1),
                });
                *prev_style = st.clone();
                *start = e % count;
            }
        };
        for seg in &segments {
            if !span_gaps {
                start = seg.start;
            }
            let end = if seg.end < start { seg.end + count } else { seg.end };
            let mut prev = start;
            let mut i = start + 1;
            while i <= end {
                let st = style(prev % count, i % count);
                if st != prev_style {
                    let ps = prev_style.clone();
                    add(start, i - 1, seg.looped, &ps, &mut start, &mut prev_style);
                }
                prev = i;
                prev_style = st;
                i += 1;
            }
            if start + 1 < i {
                let ps = prev_style.clone();
                add(start, i - 1, seg.looped, &ps, &mut start, &mut prev_style);
            }
        }
        self.styles = styles;
        self.styled = Some(result);
    }

    /// `_computeSegments` without styles: the runs of points between gaps.
    pub fn solid_segments(&self, points: &[Point]) -> Vec<Segment> {
        let count = points.len();
        if count == 0 {
            return vec![];
        }
        let span_gaps = self.options.span_gaps != SpanGaps::No;
        // findStartAndEnd
        let mut start = 0;
        let mut end = count - 1;
        if self.looped && !span_gaps {
            while start < count && !points[start].skip {
                start += 1;
            }
        }
        while start < count && points[start].skip {
            start += 1;
        }
        start %= count;
        if self.looped {
            end += start;
        }
        while end > start && points[end % count].skip {
            end -= 1;
        }
        end %= count;
        if self.options.span_gaps == SpanGaps::Yes {
            return vec![Segment { start, end, looped: self.looped, style: None }];
        }
        let max = if end < start { end + count } else { end };
        let complete_loop = self.full_loop && start == 0 && end == count - 1;
        solid_segments(points, start, max, complete_loop)
    }

    /// The whole line as one path (`LineElement.path`). Returns whether it closed into a loop.
    pub fn path(&self, c: &mut dyn Canvas, points: &[Point]) -> bool {
        let mut looped = self.looped;
        for seg in self.segments(points) {
            looped &= self.path_segment(c, points, seg, false, true);
        }
        looped
    }

    /// `pathSegment`: append one segment, optionally in reverse. Returns `segment.loop`.
    pub fn path_segment(
        &self,
        c: &mut dyn Canvas,
        points: &[Point],
        seg: Segment,
        reverse: bool,
        mut move_first: bool,
    ) -> bool {
        let count = points.len();
        if count == 0 {
            return false;
        }
        let ilen = if seg.end < seg.start { count + seg.end - seg.start } else { seg.end - seg.start };
        let mut prev: Option<&Point> = None;
        for i in 0..=ilen {
            let p = &points[(seg.start + if reverse { ilen - i } else { i }) % count];
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
        if seg.looped {
            let p = &points[(seg.start + if reverse { ilen } else { 0 }) % count];
            self.line_method(c, prev, p, reverse);
        }
        seg.looped
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

    /// `LineElement.interpolate`: the point on this line where `prop` equals `value`. `None` when
    /// there is no such point, or when there are several (Chart.js then returns an array, which
    /// `lineTo` ignores).
    pub fn interpolate(&self, points: &[Point], value: f64, prop: Prop) -> Option<(f64, f64)> {
        let bounds = Bounds { prop, start: value, end: value };
        let segs = bound_segments(&self.segments(points), points, Some(bounds));
        let mut result = Vec::new();
        for s in segs {
            let (p1, p2) = (&points[s.start], &points[s.end]);
            if s.start == s.end {
                result.push((p1.x, p1.y));
                continue;
            }
            let t = ((value - p1.get(prop)) / (p2.get(prop) - p1.get(prop))).abs();
            let lerp = |a: (f64, f64), b: (f64, f64), t: f64| (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            let pt = match self.options.stepped {
                Stepped::No if self.uses_curve() => {
                    let (a, cp1, cp2, b) =
                        ((p1.x, p1.y), (p1.cp2x, p1.cp2y), (p2.cp1x, p2.cp1y), (p2.x, p2.y));
                    let (q1, q2, q3) = (lerp(a, cp1, t), lerp(cp1, cp2, t), lerp(cp2, b, t));
                    lerp(lerp(q1, q2, t), lerp(q2, q3, t), t)
                }
                Stepped::No => lerp((p1.x, p1.y), (p2.x, p2.y), t),
                mode => {
                    let x = p1.x + t * (p2.x - p1.x);
                    let y = match mode {
                        Stepped::Middle => {
                            if t < 0.5 {
                                p1.y
                            } else {
                                p2.y
                            }
                        }
                        Stepped::After => {
                            if t < 1.0 {
                                p1.y
                            } else {
                                p2.y
                            }
                        }
                        _ => {
                            if t > 0.0 {
                                p2.y
                            } else {
                                p1.y
                            }
                        }
                    };
                    (x, y)
                }
            };
            let mut pt = pt;
            match prop {
                Prop::X => pt.0 = value,
                Prop::Y => pt.1 = value,
                Prop::Angle => {}
            }
            result.push(pt);
        }
        if result.len() == 1 { result.pop() } else { None }
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
        c.set_stroke_style(o.border_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
        if self.styled.is_some() {
            // strokePathDirect: each segment on its own, in its style.
            for seg in self.segments(points) {
                let st = seg.style.and_then(|i| self.styles.get(i));
                let dash = st.and_then(|s| s.dash.as_ref()).unwrap_or(&o.dash);
                c.set_line_cap(st.and_then(|s| s.cap).unwrap_or(o.cap));
                c.set_line_dash(&dash.iter().map(|d| *d as f32).collect::<Vec<f32>>());
                c.set_line_dash_offset(st.and_then(|s| s.dash_offset).unwrap_or(o.dash_offset) as f32);
                c.set_line_join(st.and_then(|s| s.join).unwrap_or(o.join));
                c.set_line_width(st.and_then(|s| s.border_width).unwrap_or(o.border_width) as f32);
                c.set_stroke_style(
                    st.and_then(|s| s.border_color.clone())
                        .or_else(|| o.border_color.clone())
                        .unwrap_or(Paint::Solid(Color::TRANSPARENT)),
                );
                c.begin_path();
                if self.path_segment(c, points, seg, false, true) {
                    c.close_path();
                }
                c.stroke();
            }
            c.restore();
            return;
        }
        // Chart.js strokes one Path2D holding every segment, closed if it loops.
        c.begin_path();
        if self.path(c, points) {
            c.close_path();
        }
        c.stroke();
        c.restore();
    }
}

/// `solidSegments`.
fn solid_segments(points: &[Point], start: usize, max: usize, mut looped: bool) -> Vec<Segment> {
    let count = points.len();
    let mut result = Vec::new();
    let mut seg_start: Option<usize> = Some(start);
    let mut last: Option<usize> = Some(start);
    let mut prev_skip = points[start].skip;
    for e in start + 1..=max {
        let cur = &points[e % count];
        if cur.skip || cur.stop {
            if !prev_skip {
                looped = false;
                if let Some(s) = seg_start {
                    result.push(Segment { start: s % count, end: (e - 1) % count, looped, style: None });
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
        result.push(Segment { start: s % count, end: l % count, looped, style: None });
    }
    result
}

/// `{property, start, end}` bounds for segment clipping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub prop: Prop,
    pub start: f64,
    pub end: f64,
}

fn angle_diff(a: f64, b: f64) -> f64 {
    use std::f64::consts::{PI, TAU};
    (a - b + TAU + PI) % TAU - PI
}

fn between(prop: Prop, v: f64, a: f64, b: f64) -> bool {
    match prop {
        Prop::Angle => crate::arc::angle_between(v, a, b, false),
        _ => crate::math::is_between(v, a, b),
    }
}

fn compare(prop: Prop, a: f64, b: f64) -> f64 {
    match prop {
        Prop::Angle => angle_diff(a, b),
        _ => a - b,
    }
}

fn normalize(prop: Prop, v: f64) -> f64 {
    match prop {
        Prop::Angle => crate::arc::normalize_angle(v),
        _ => v,
    }
}

/// `_boundSegment`: the parts of `seg` inside `bounds`.
pub fn bound_segment(seg: Segment, points: &[Point], bounds: Option<Bounds>) -> Vec<Segment> {
    let Some(b) = bounds else { return vec![seg] };
    let count = points.len();
    let prop = b.prop;
    // getSegment
    let (mut start, mut end) = (seg.start, seg.end);
    if seg.looped {
        start += count;
        end += count;
        for _ in 0..count {
            if !between(prop, normalize(prop, points[start % count].get(prop)), b.start, b.end) {
                break;
            }
            start -= 1;
            end -= 1;
        }
        start %= count;
        end %= count;
    }
    if end < start {
        end += count;
    }
    let normalize_seg = |s: usize, e: usize| Segment {
        start: s % count,
        end: e % count,
        looped: seg.looped && (e - s + 1).is_multiple_of(count),
        style: seg.style,
    };
    let mut result = Vec::new();
    let mut inside;
    let mut sub_start: Option<usize> = None;
    let mut prev_value: Option<f64> = None;
    let mut prev = start;
    for i in start..=end {
        let point = &points[i % count];
        if point.skip {
            continue;
        }
        let value = normalize(prop, point.get(prop));
        if Some(value) == prev_value {
            continue;
        }
        inside = between(prop, value, b.start, b.end);
        let start_is_before = prev_value
            .is_some_and(|pv| between(prop, b.start, pv, value) && compare(prop, b.start, pv) != 0.0);
        let end_is_before = compare(prop, b.end, value) == 0.0
            || prev_value.is_some_and(|pv| between(prop, b.end, pv, value));
        if sub_start.is_none() && (inside || start_is_before) {
            sub_start = Some(if compare(prop, value, b.start) == 0.0 { i } else { prev });
        }
        if let Some(s) = sub_start
            && (!inside || end_is_before)
        {
            result.push(normalize_seg(s, i));
            sub_start = None;
        }
        prev = i;
        prev_value = Some(value);
    }
    if let Some(s) = sub_start {
        result.push(normalize_seg(s, end));
    }
    result
}

/// `_boundSegments`.
pub fn bound_segments(segments: &[Segment], points: &[Point], bounds: Option<Bounds>) -> Vec<Segment> {
    segments.iter().flat_map(|s| bound_segment(*s, points, bounds)).collect()
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
    pub background_color: Option<Paint>,
    pub border_color: Option<Paint>,
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
            c.set_fill_style(self.options.border_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
            c.fill_with_rule(FillRule::EvenOdd);
        }
        c.begin_path();
        add(c, inflate_rect(inner, self.inflate_amount, None));
        c.set_fill_style(self.options.background_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
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
