//! The `filler` plugin (area fills): a port of Chart.js `plugin.filler/*`.

use kittychart_canvas::{Canvas, Color, FillRule, Paint};
use serde_json::Value;

use crate::element::{Bounds, Line, Point, Prop, Segment, bound_segment, bound_segments};
use crate::options::as_num;
use crate::scale::ChartArea;

/// A decoded `fill` option (`_decodeFill`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FillTarget {
    Origin,
    Start,
    End,
    Value(f64),
    Dataset(usize),
    Stack,
    Shape,
}

/// `fill` for one dataset, plus optional above/below colors.
#[derive(Debug, Clone, PartialEq)]
pub struct FillSpec {
    pub target: Option<FillTarget>,
    pub above: Option<Color>,
    pub below: Option<Color>,
}

/// `parseFillOption` + `_decodeFill`.
pub fn decode_fill(fill: Option<&Value>, has_background: bool, index: usize, count: usize) -> FillSpec {
    let color = |k: &str| fill.and_then(|f| f.get(k)).and_then(Value::as_str).and_then(crate::color::parse);
    let (above, below) = (color("above"), color("below"));
    // valueOrDefault(fillOption && fillOption.target, fillOption)
    let raw = match fill {
        Some(Value::Object(o)) => o.get("target").cloned().or_else(|| Some(Value::Object(o.clone()))),
        other => other.cloned(),
    };
    let target = match raw {
        None => has_background.then_some(FillTarget::Origin),
        Some(Value::Null | Value::Bool(false)) => None,
        Some(Value::Bool(true)) => Some(FillTarget::Origin),
        Some(Value::Object(o)) => {
            o.get("value").and_then(as_num).filter(|v| !v.is_nan()).map(FillTarget::Value)
        }
        Some(Value::Number(n)) => n.as_f64().and_then(|t| absolute_target(t, index, count)),
        Some(Value::String(s)) => match s.as_str() {
            "origin" => Some(FillTarget::Origin),
            "start" => Some(FillTarget::Start),
            "end" => Some(FillTarget::End),
            "stack" => Some(FillTarget::Stack),
            "shape" => Some(FillTarget::Shape),
            other => match other.parse::<f64>() {
                Ok(t) if t.fract() == 0.0 => {
                    let relative = other.starts_with('-') || other.starts_with('+');
                    let t = if relative { index as f64 + t } else { t };
                    absolute_target(t, index, count)
                }
                _ => None,
            },
        },
        Some(_) => None,
    };
    FillSpec { target, above, below }
}

fn absolute_target(t: f64, index: usize, count: usize) -> Option<FillTarget> {
    if t.fract() != 0.0 || t < 0.0 || t >= count as f64 || t == index as f64 {
        return None;
    }
    Some(FillTarget::Dataset(t as usize))
}

/// `_resolveTarget`: follow dataset targets past hidden datasets when `propagate` is on.
pub fn resolve_target(
    fills: &[Option<FillTarget>],
    visible: &[bool],
    index: usize,
    propagate: bool,
) -> Option<FillTarget> {
    let mut fill = fills[index];
    if !propagate {
        return fill;
    }
    let mut visited = vec![index];
    while let Some(f) = fill {
        let FillTarget::Dataset(i) = f else { return Some(f) };
        if visited.contains(&i) {
            return None;
        }
        if *visible.get(i)? {
            return Some(f);
        }
        visited.push(i);
        fill = fills[i];
    }
    None
}

/// What to fill toward.
#[derive(Debug, Clone)]
pub enum TargetGeom {
    /// Another line (dataset or stack line) or a boundary line built from the source.
    Line(Box<Line>, Vec<Point>),
    /// A circle (radial scale with circular grid).
    Arc { x: f64, y: f64, radius: f64 },
    /// `fill: 'shape'`: the line's own outline.
    Shape,
}

impl TargetGeom {
    fn segments(&self) -> Option<Vec<Segment>> {
        match self {
            TargetGeom::Line(l, p) => Some(l.segments(p)),
            _ => None,
        }
    }

    fn path_segment(&self, c: &mut dyn Canvas, tgt: &PartTarget, move_first: bool, reverse: bool) -> bool {
        match (self, tgt) {
            (TargetGeom::Line(l, p), PartTarget::Segment(s)) => l.path_segment(c, p, *s, reverse, move_first),
            (TargetGeom::Arc { x, y, radius }, PartTarget::Bounds(b)) => {
                let (start, end) = b.map_or((0.0, std::f64::consts::TAU), |b| (b.start, b.end));
                c.arc(*x as f32, *y as f32, *radius as f32, end as f32, start as f32, true);
                true
            }
            _ => false,
        }
    }

    fn interpolate(&self, value: f64, prop: Prop) -> Option<(f64, f64)> {
        match self {
            TargetGeom::Line(l, p) => l.interpolate(p, value, prop),
            TargetGeom::Arc { x, y, radius } => Some((x + value.cos() * radius, y + value.sin() * radius)),
            TargetGeom::Shape => None,
        }
    }

    fn first(&self) -> Option<(f64, f64)> {
        match self {
            TargetGeom::Line(l, p) => l.segments(p).first().map(|s| (p[s.start].x, p[s.start].y)),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum PartTarget {
    Segment(Segment),
    Bounds(Option<Bounds>),
}

#[derive(Debug, Clone, Copy)]
struct Part {
    source: Segment,
    target: PartTarget,
    start: f64,
    end: f64,
}

fn find_segment_end(start: usize, mut end: usize, points: &[Point]) -> usize {
    while end > start {
        let p = &points[end];
        if !p.x.is_nan() && !p.y.is_nan() {
            break;
        }
        end -= 1;
    }
    end
}

fn get_bounds(prop: Prop, start: f64, end: f64, looped: bool) -> Option<Bounds> {
    if looped {
        return None;
    }
    let (s, e) = match prop {
        Prop::Angle => (crate::arc::normalize_angle(start), crate::arc::normalize_angle(end)),
        _ => (start, end),
    };
    Some(Bounds { prop, start: s, end: e })
}

/// `_segments(line, target, property)`.
fn parts(line: &Line, points: &[Point], target: &TargetGeom, prop: Prop) -> Vec<Part> {
    let mut out = Vec::new();
    let tsegs_all = target.segments();
    for seg in line.segments(points) {
        let end = find_segment_end(seg.start, seg.end, points);
        let (sv, ev) = (points[seg.start].get(prop), points[end].get(prop));
        let bounds = get_bounds(prop, sv, ev, seg.looped);
        let (Some(tsegs), TargetGeom::Line(_, tpoints)) = (&tsegs_all, target) else {
            out.push(Part { source: seg, target: PartTarget::Bounds(bounds), start: sv, end: ev });
            continue;
        };
        for tgt in bound_segments(tsegs, tpoints, bounds) {
            let sub = get_bounds(prop, tpoints[tgt.start].get(prop), tpoints[tgt.end].get(prop), tgt.looped);
            let edge = |pick: fn(&Bounds) -> f64, f: fn(f64, f64) -> f64| match (bounds, sub) {
                (Some(a), Some(b)) => f(pick(&a), pick(&b)),
                (Some(a), None) => pick(&a),
                (None, Some(b)) => pick(&b),
                (None, None) => 0.0,
            };
            for fs in bound_segment(seg, points, sub) {
                out.push(Part {
                    source: fs,
                    target: PartTarget::Segment(tgt),
                    start: edge(|b| b.start, f64::max),
                    end: edge(|b| b.end, f64::min),
                });
            }
        }
    }
    out
}

/// Everything `_drawfill` needs about one dataset.
#[derive(Debug)]
pub struct FillJob<'a> {
    pub line: &'a Line,
    pub points: &'a [Point],
    pub target: TargetGeom,
    pub above: Paint,
    pub below: Paint,
    /// `line._loop ? 'angle' : indexAxis`.
    pub prop: Prop,
    pub chart_area: ChartArea,
    /// The dataset's clip rectangle, if any.
    pub clip: Option<ChartArea>,
    /// `clipToIndexBounds(area, iScale)`.
    pub index_bounds: ChartArea,
}

/// `_drawfill` → `doFill`.
pub fn draw_fill(c: &mut dyn Canvas, job: &FillJob<'_>) {
    if job.points.is_empty() {
        return;
    }
    let b = job.index_bounds;
    crate::draw::clip_area(c, b.left, b.top, b.right, b.bottom);
    c.save();
    let mut fill_color = job.below.clone();
    if job.below != job.above && matches!(job.target, TargetGeom::Line(..)) {
        let area = job.chart_area;
        match job.prop {
            Prop::X => {
                clip_to_target(c, &job.target, area.top, true);
                fill(c, job, &job.above);
                c.restore();
                c.save();
                clip_to_target(c, &job.target, area.bottom, true);
            }
            Prop::Y => {
                clip_to_target(c, &job.target, area.left, false);
                fill(c, job, &job.below);
                c.restore();
                c.save();
                clip_to_target(c, &job.target, area.right, false);
                fill_color = job.above.clone();
            }
            Prop::Angle => {}
        }
    }
    fill(c, job, &fill_color);
    c.restore();
    c.restore();
}

/// `clipVertical` / `clipHorizontal`: clip to the region between the target line and an edge.
fn clip_to_target(c: &mut dyn Canvas, target: &TargetGeom, edge: f64, vertical: bool) {
    let TargetGeom::Line(l, points) = target else { return };
    let f = |v: f64| v as f32;
    let to = |c: &mut dyn Canvas, p: &Point| {
        if vertical { c.line_to(f(p.x), f(edge)) } else { c.line_to(f(edge), f(p.y)) }
    };
    let mut first = true;
    let mut line_loop = false;
    c.begin_path();
    for seg in l.segments(points) {
        let fp = &points[seg.start];
        let lp = &points[find_segment_end(seg.start, seg.end, points)];
        if first {
            c.move_to(f(fp.x), f(fp.y));
            first = false;
        } else {
            to(c, fp);
            c.line_to(f(fp.x), f(fp.y));
        }
        line_loop = l.path_segment(c, points, seg, false, line_loop);
        if line_loop {
            c.close_path();
        } else {
            to(c, lp);
        }
    }
    if let Some((x, y)) = target.first() {
        if vertical { c.line_to(f(x), f(edge)) } else { c.line_to(f(edge), f(y)) }
    }
    c.close_path();
    c.clip();
}

fn fill(c: &mut dyn Canvas, job: &FillJob<'_>, color: &Paint) {
    let shape = matches!(job.target, TargetGeom::Shape);
    for part in parts(job.line, job.points, &job.target, job.prop) {
        c.save();
        // A segment's own `backgroundColor` (segment styling) wins over the dataset's.
        let style = part.source.style.and_then(|i| job.line.styles.get(i));
        c.set_fill_style(style.and_then(|s| s.background_color.clone()).unwrap_or_else(|| color.clone()));
        if !shape {
            clip_bounds(c, &job.chart_area, job.clip, get_bounds(job.prop, part.start, part.end, false));
        }
        c.begin_path();
        let line_loop = job.line.path_segment(c, job.points, part.source, false, true);
        let mut looped = false;
        if !shape {
            if line_loop {
                c.close_path();
            } else if let Some((x, y)) = job.target.interpolate(part.end, job.prop) {
                c.line_to(x as f32, y as f32);
            }
            let target_loop = job.target.path_segment(c, &part.target, line_loop, true);
            looped = line_loop && target_loop;
            if !looped && let Some((x, y)) = job.target.interpolate(part.start, job.prop) {
                c.line_to(x as f32, y as f32);
            }
        }
        c.close_path();
        c.fill_with_rule(if looped { FillRule::EvenOdd } else { FillRule::NonZero });
        c.restore();
    }
}

/// `clipBounds`: for x/y fills, clip to the part's span within the chart area.
fn clip_bounds(c: &mut dyn Canvas, area: &ChartArea, clip: Option<ChartArea>, bounds: Option<Bounds>) {
    let Some(b) = bounds else { return };
    let (mut l, mut t, mut r, mut btm) = match b.prop {
        Prop::X => (b.start, area.top, b.end, area.bottom),
        Prop::Y => (area.left, b.start, area.right, b.end),
        Prop::Angle => return,
    };
    if let Some(cl) = clip {
        l = l.max(cl.left);
        r = r.min(cl.right);
        t = t.max(cl.top);
        btm = btm.min(cl.bottom);
    }
    c.begin_path();
    c.rect(l as f32, t as f32, (r - l) as f32, (btm - t) as f32);
    c.clip();
}

/// `_pointsFromSegments`: a horizontal (or vertical) boundary spanning each source segment.
pub fn boundary_points(line: &Line, points: &[Point], x: Option<f64>, y: Option<f64>) -> Vec<Point> {
    let mut out = Vec::new();
    for seg in line.segments(points) {
        let end = find_segment_end(seg.start, seg.end, points);
        let (first, last) = (&points[seg.start], &points[end]);
        if let Some(y) = y {
            out.push(Point::at(first.x, y, 0.0));
            out.push(Point::at(last.x, y, 0.0));
        } else if let Some(x) = x {
            out.push(Point::at(x, first.y, 0.0));
            out.push(Point::at(x, last.y, 0.0));
        }
    }
    out
}

/// `_buildStackLine`: the outline of the lines stacked below `points`.
pub fn stack_line(
    line: &Line,
    points: &[Point],
    below: &[(&Line, &[Point])],
    scale_bottom: f64,
) -> Vec<Point> {
    let boundary_pts = boundary_points(line, points, None, Some(scale_bottom));
    let boundary = Line::new(crate::element::LineOptions::plain());
    let mut lines: Vec<(&Line, &[Point])> = below.to_vec();
    lines.push((&boundary, &boundary_pts));
    let mut out = Vec::new();
    for seg in line.segments(points) {
        for sp in &points[seg.start..=seg.end] {
            let mut postponed = Vec::new();
            for (bl, bp) in &lines {
                let Some((x, y)) = bl.interpolate(bp, sp.x, Prop::X) else { continue };
                // findPoint: is it the first/last point of a segment of that line?
                let (mut first, mut last) = (false, false);
                for s in bl.segments(bp) {
                    let (fv, lv) = (bp[s.start].x, bp[s.end].x);
                    if crate::math::is_between(x, fv, lv) {
                        first = x == fv;
                        last = x == lv;
                        break;
                    }
                }
                if first && last {
                    continue;
                }
                let p = Point::at(x, y, 0.0);
                if first {
                    postponed.insert(0, p);
                } else {
                    out.push(p);
                    if !last {
                        break;
                    }
                }
            }
            out.extend(postponed);
        }
    }
    out
}
