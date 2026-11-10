//! Hit-testing: which elements an event position selects (Chart.js `Interaction.modes`).
//!
//! A port of `core.interaction.js` and the elements' `inRange`/`getCenterPoint` methods. The
//! modes are pure functions over the chart's last-drawn geometry, so they work the same for
//! tooltips, hover styles and clicks, and can be tested without a terminal.

use crate::arc::{Arc, angle_between};
use crate::chart::Chart;
use crate::element::{Bar, Point};
use crate::scale::ChartArea;

/// An element selected by an interaction: its dataset and data index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActiveElement {
    pub dataset: usize,
    pub index: usize,
}

/// `interaction.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Index,
    Dataset,
    Point,
    Nearest,
    X,
    Y,
}

impl Mode {
    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s {
            "index" => Mode::Index,
            "dataset" => Mode::Dataset,
            "point" => Mode::Point,
            "nearest" => Mode::Nearest,
            "x" => Mode::X,
            "y" => Mode::Y,
            _ => return None,
        })
    }
}

/// `interaction` options (or a plugin's, such as `plugins.tooltip`, which override them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InteractionOptions {
    pub mode: Mode,
    pub intersect: bool,
    /// `axis`: `None` means the mode's default (`x` for index, `xy` otherwise).
    pub axis: Option<Axis>,
    pub include_invisible: bool,
}

impl Default for InteractionOptions {
    /// Chart.js's global defaults: `{mode: 'nearest', intersect: true}`.
    fn default() -> Self {
        InteractionOptions { mode: Mode::Nearest, intersect: true, axis: None, include_invisible: false }
    }
}

/// Which coordinates a distance or range uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
    XY,
    R,
}

impl Axis {
    pub fn parse(s: &str) -> Option<Axis> {
        Some(match s {
            "x" => Axis::X,
            "y" => Axis::Y,
            "xy" => Axis::XY,
            "r" => Axis::R,
            _ => return None,
        })
    }
}

/// A hit-testable element, borrowed from the chart.
#[derive(Debug, Clone, Copy)]
enum El<'a> {
    Point(&'a Point),
    Bar(&'a Bar),
    Arc(&'a Arc),
}

impl El<'_> {
    fn skip(&self) -> bool {
        match self {
            El::Point(p) => p.skip,
            _ => false,
        }
    }

    fn in_range(&self, x: f64, y: f64) -> bool {
        match self {
            El::Point(p) => {
                (x - p.x).powi(2) + (y - p.y).powi(2) < (p.options.hit_radius + p.options.radius).powi(2)
            }
            El::Bar(b) => bar_in_range(b, Some(x), Some(y)),
            El::Arc(a) => arc_in_range(a, x, y),
        }
    }

    fn in_x_range(&self, x: f64) -> Option<bool> {
        match self {
            El::Point(p) => Some((x - p.x).abs() < p.options.radius + p.options.hit_radius),
            El::Bar(b) => Some(bar_in_range(b, Some(x), None)),
            El::Arc(_) => None,
        }
    }

    fn in_y_range(&self, y: f64) -> Option<bool> {
        match self {
            El::Point(p) => Some((y - p.y).abs() < p.options.radius + p.options.hit_radius),
            El::Bar(b) => Some(bar_in_range(b, None, Some(y))),
            El::Arc(_) => None,
        }
    }

    fn center(&self) -> (f64, f64) {
        match self {
            El::Point(p) => (p.x, p.y),
            El::Bar(b) => {
                if b.horizontal {
                    ((b.x + b.base) / 2.0, b.y)
                } else {
                    (b.x, (b.y + b.base) / 2.0)
                }
            }
            El::Arc(a) => arc_center(a),
        }
    }
}

fn bar_in_range(b: &Bar, x: Option<f64>, y: Option<f64>) -> bool {
    let (l, t, r, bt) = b.bounds();
    x.is_none_or(|x| is_between(x, l, r)) && y.is_none_or(|y| is_between(y, t, bt))
}

fn arc_in_range(a: &Arc, x: f64, y: f64) -> bool {
    let (angle, distance) = angle_from_point((a.x, a.y), (x, y));
    let r_adjust = (a.options.spacing + a.options.border_width) / 2.0;
    let circumference = if a.circumference.is_nan() { a.end_angle - a.start_angle } else { a.circumference };
    let non_zero_between =
        angle_between(angle, a.start_angle, a.end_angle, false) && a.start_angle != a.end_angle;
    let between_angles = circumference >= std::f64::consts::TAU || non_zero_between;
    between_angles && is_between(distance, a.inner_radius + r_adjust, a.outer_radius + r_adjust)
}

/// `ArcElement.getCenterPoint` (also its tooltip position).
pub fn arc_center(a: &Arc) -> (f64, f64) {
    let half_angle = (a.start_angle + a.end_angle) / 2.0;
    let half_radius = (a.inner_radius + a.outer_radius + a.options.spacing + a.options.offset) / 2.0;
    (a.x + half_angle.cos() * half_radius, a.y + half_angle.sin() * half_radius)
}

/// `getAngleFromPoint`: angle in (-π/2, 3π/2] and distance.
fn angle_from_point(center: (f64, f64), p: (f64, f64)) -> (f64, f64) {
    let (dx, dy) = (p.0 - center.0, p.1 - center.1);
    let mut angle = dy.atan2(dx);
    if angle < -0.5 * std::f64::consts::PI {
        angle += std::f64::consts::TAU;
    }
    (angle, (dx * dx + dy * dy).sqrt())
}

/// `_isBetween` with Chart.js's epsilon.
fn is_between(v: f64, a: f64, b: f64) -> bool {
    v >= a.min(b) - 1e-6 && v <= a.max(b) + 1e-6
}

/// `_isPointInArea` (margin 0 means 0.5, as in Chart.js).
fn point_in_area(p: (f64, f64), area: &ChartArea, margin: f64) -> bool {
    let m = if margin == 0.0 { 0.5 } else { margin };
    p.0 > area.left - m && p.0 < area.right + m && p.1 > area.top - m && p.1 < area.bottom + m
}

/// An element's tooltip anchor (`tooltipPosition`): the point, the bar's tip, the arc's center.
pub fn tooltip_position(chart: &Chart, e: ActiveElement) -> Option<(f64, f64)> {
    let m = chart.metas.iter().position(|m| m.index == e.dataset)?;
    let meta = &chart.metas[m];
    if let Some(p) = meta.points.get(e.index).filter(|_| meta.bars.is_empty() && meta.arcs.is_empty()) {
        return Some((p.x, p.y));
    }
    if let Some(b) = meta.bars.get(e.index) {
        return Some((b.x, b.y));
    }
    meta.arcs.get(e.index).map(arc_center)
}

impl Chart {
    /// The data elements of each visible dataset, in `getSortedVisibleDatasetMetas` order.
    fn hit_elements(&self, include_invisible: bool) -> Vec<(usize, Vec<El<'_>>)> {
        self.sorted_metas(!include_invisible)
            .into_iter()
            .map(|m| {
                let meta = &self.metas[m];
                let els: Vec<El<'_>> = if !meta.bars.is_empty() {
                    meta.bars.iter().map(El::Bar).collect()
                } else if !meta.arcs.is_empty() {
                    meta.arcs.iter().map(El::Arc).collect()
                } else {
                    meta.points.iter().map(El::Point).collect()
                };
                (meta.index, els)
            })
            .collect()
    }

    /// `chart.isPointInArea`.
    fn point_in_chart_area(&self, p: (f64, f64)) -> bool {
        point_in_area(p, &self.chart_area, self.min_padding)
    }

    /// `getElementsAtEventForMode`: the elements an event at `(x, y)` (CSS pixels, relative to
    /// the chart) selects with `opts`.
    pub fn elements_at(&self, x: f64, y: f64, opts: &InteractionOptions) -> Vec<ActiveElement> {
        let inv = opts.include_invisible;
        match opts.mode {
            Mode::Index => {
                let axis = opts.axis.unwrap_or(Axis::X);
                let items = if opts.intersect {
                    self.intersect_items(x, y, inv)
                } else {
                    self.nearest_items(x, y, axis, false, inv)
                };
                let Some(first) = items.first() else { return vec![] };
                self.hit_elements(false)
                    .into_iter()
                    .filter_map(|(ds, els)| {
                        els.get(first.index)
                            .filter(|e| !e.skip())
                            .map(|_| ActiveElement { dataset: ds, index: first.index })
                    })
                    .collect()
            }
            Mode::Dataset => {
                let axis = opts.axis.unwrap_or(Axis::XY);
                let items = if opts.intersect {
                    self.intersect_items(x, y, inv)
                } else {
                    self.nearest_items(x, y, axis, false, inv)
                };
                let Some(first) = items.first() else { return vec![] };
                let n = self
                    .metas
                    .iter()
                    .find(|m| m.index == first.dataset)
                    .map_or(0, |m| m.points.len().max(m.bars.len()).max(m.arcs.len()));
                (0..n).map(|index| ActiveElement { dataset: first.dataset, index }).collect()
            }
            Mode::Point => self.intersect_items(x, y, inv),
            Mode::Nearest => self.nearest_items(x, y, opts.axis.unwrap_or(Axis::XY), opts.intersect, inv),
            Mode::X => self.axis_items(x, y, Axis::X, opts.intersect),
            Mode::Y => self.axis_items(x, y, Axis::Y, opts.intersect),
        }
    }

    fn intersect_items(&self, x: f64, y: f64, include_invisible: bool) -> Vec<ActiveElement> {
        if !include_invisible && !self.point_in_chart_area((x, y)) {
            return vec![];
        }
        let mut items = Vec::new();
        for (ds, els) in self.hit_elements(include_invisible) {
            for (i, e) in els.iter().enumerate() {
                if e.skip()
                    || (!include_invisible && !point_in_area(e.center_for_area(), &self.chart_area, 0.0))
                {
                    continue;
                }
                if e.in_range(x, y) {
                    items.push(ActiveElement { dataset: ds, index: i });
                }
            }
        }
        items
    }

    fn nearest_items(&self, x: f64, y: f64, axis: Axis, intersect: bool, inv: bool) -> Vec<ActiveElement> {
        if !inv && !self.point_in_chart_area((x, y)) {
            return vec![];
        }
        if axis == Axis::R && !intersect {
            let mut items = Vec::new();
            for (ds, els) in self.hit_elements(inv) {
                for (i, e) in els.iter().enumerate() {
                    if let El::Arc(a) = e {
                        let (angle, _) = angle_from_point((a.x, a.y), (x, y));
                        if angle_between(angle, a.start_angle, a.end_angle, false) {
                            items.push(ActiveElement { dataset: ds, index: i });
                        }
                    }
                }
            }
            return items;
        }
        let dist = |a: (f64, f64), b: (f64, f64)| {
            let dx = if matches!(axis, Axis::X | Axis::XY) { (a.0 - b.0).abs() } else { 0.0 };
            let dy = if matches!(axis, Axis::Y | Axis::XY) { (a.1 - b.1).abs() } else { 0.0 };
            (dx * dx + dy * dy).sqrt()
        };
        let mut items = Vec::new();
        let mut min = f64::INFINITY;
        for (ds, els) in self.hit_elements(inv) {
            for (i, e) in els.iter().enumerate() {
                if e.skip() {
                    continue;
                }
                let in_range = e.in_range(x, y);
                if intersect && !in_range {
                    continue;
                }
                let center = e.center();
                if !(inv || self.point_in_chart_area(center)) && !in_range {
                    continue;
                }
                let d = dist((x, y), center);
                if d < min {
                    items = vec![ActiveElement { dataset: ds, index: i }];
                    min = d;
                } else if d == min {
                    items.push(ActiveElement { dataset: ds, index: i });
                }
            }
        }
        items
    }

    fn axis_items(&self, x: f64, y: f64, axis: Axis, intersect: bool) -> Vec<ActiveElement> {
        let mut items = Vec::new();
        let mut intersects = false;
        for (ds, els) in self.hit_elements(false) {
            for (i, e) in els.iter().enumerate() {
                if e.skip() {
                    continue;
                }
                let hit = match axis {
                    Axis::X => e.in_x_range(x),
                    _ => e.in_y_range(y),
                };
                if hit == Some(true) {
                    items.push(ActiveElement { dataset: ds, index: i });
                    intersects = intersects || e.in_range(x, y);
                }
            }
        }
        if intersect && !intersects { vec![] } else { items }
    }
}

impl El<'_> {
    /// The position `_isPointInArea(element, …)` tests: the element's own `x`/`y`.
    fn center_for_area(&self) -> (f64, f64) {
        match self {
            El::Point(p) => (p.x, p.y),
            El::Bar(b) => (b.x, b.y),
            El::Arc(a) => (a.x, a.y),
        }
    }
}
