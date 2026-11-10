//! Box layout: a port of Chart.js `core.layouts.js`.
//!
//! Boxes (scales, title, legend) are fitted around the chart area in several passes. Each box
//! reports its size for the space it is offered, and the area shrinks until everything fits.

use std::collections::HashMap;

use kittychart_canvas::Canvas;

use crate::options::Padding;
use crate::scale::{ChartArea, Position};

/// Anything placed by the layout engine.
pub trait LayoutItem {
    fn position(&self) -> Position;
    fn weight(&self) -> f64;
    fn full_size(&self) -> bool;
    fn is_horizontal(&self) -> bool;
    /// `options.display !== false` (used to count vertical boxes).
    fn displayed(&self) -> bool;
    fn stack(&self) -> Option<String> {
        None
    }
    fn stack_weight(&self) -> f64 {
        1.0
    }
    fn update(&mut self, max_width: f64, max_height: f64, margins: Padding, c: &mut dyn Canvas);
    /// `getPadding()`, the overhang a box needs from the chart edges (scales only).
    fn padding(&self) -> Option<Padding> {
        None
    }
    fn width(&self) -> f64;
    fn height(&self) -> f64;
    fn set_box(&mut self, left: f64, top: f64, width: f64, height: f64);
    fn right(&self) -> f64;
    fn bottom(&self) -> f64;
}

#[derive(Debug, Clone)]
struct Wrap {
    index: usize,
    pos: Position,
    horizontal: bool,
    weight: f64,
    full_size: bool,
    stack: Option<String>,
    stack_weight: f64,
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Default)]
struct Stack {
    count: f64,
    placed: f64,
    weight: f64,
    size: f64,
    start: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
struct Params {
    outer_width: f64,
    outer_height: f64,
    padding: Padding,
    available_width: f64,
    available_height: f64,
    v_box_max_width: f64,
    h_box_max_height: f64,
}

#[derive(Debug, Clone, Default)]
struct Area {
    max_padding: Padding,
    w: f64,
    h: f64,
    x: f64,
    y: f64,
    top: f64,
    left: f64,
    bottom: f64,
    right: f64,
}

impl Area {
    fn side(&mut self, p: Position) -> &mut f64 {
        match p {
            Position::Top => &mut self.top,
            Position::Left => &mut self.left,
            Position::Bottom => &mut self.bottom,
            Position::Right => &mut self.right,
        }
    }
}

fn side(p: &Padding, pos: Position) -> f64 {
    match pos {
        Position::Top => p.top,
        Position::Left => p.left,
        Position::Bottom => p.bottom,
        Position::Right => p.right,
    }
}

fn sort_by_weight(v: &mut [Wrap], reverse: bool) {
    v.sort_by(|a, b| {
        let (v0, v1) = if reverse { (b, a) } else { (a, b) };
        if v0.weight == v1.weight {
            v0.index.cmp(&v1.index)
        } else {
            v0.weight.partial_cmp(&v1.weight).unwrap_or(std::cmp::Ordering::Equal)
        }
    });
}

/// Run the layout. `min_padding` is the datasets' max overflow (`layout.autoPadding`).
pub fn update(
    items: &mut [&mut dyn LayoutItem],
    width: f64,
    height: f64,
    padding: Padding,
    min_padding: f64,
    c: &mut dyn Canvas,
) -> ChartArea {
    let available_width = (width - padding.width()).max(0.0);
    let available_height = (height - padding.height()).max(0.0);

    let wraps: Vec<Wrap> = items
        .iter()
        .enumerate()
        .map(|(i, b)| Wrap {
            index: i,
            pos: b.position(),
            horizontal: b.is_horizontal(),
            weight: b.weight(),
            full_size: b.full_size(),
            stack: b.stack().map(|s| format!("{:?}{s}", b.position())),
            stack_weight: b.stack_weight(),
            width: 0.0,
            height: 0.0,
        })
        .collect();

    let by_pos = |p: Position, rev: bool| {
        let mut v: Vec<Wrap> = wraps.iter().filter(|w| w.pos == p).cloned().collect();
        sort_by_weight(&mut v, rev);
        v
    };
    let mut full: Vec<Wrap> = wraps.iter().filter(|w| w.full_size).cloned().collect();
    sort_by_weight(&mut full, true);
    let left = by_pos(Position::Left, true);
    let right = by_pos(Position::Right, false);
    let top = by_pos(Position::Top, true);
    let bottom = by_pos(Position::Bottom, false);
    let left_and_top: Vec<Wrap> = left.iter().chain(top.iter()).cloned().collect();
    let right_and_bottom: Vec<Wrap> = right.iter().chain(bottom.iter()).cloned().collect();
    let vertical: Vec<Wrap> = left.iter().chain(right.iter()).cloned().collect();
    let horizontal: Vec<Wrap> = top.iter().chain(bottom.iter()).cloned().collect();

    let visible_vertical = vertical.iter().filter(|w| items[w.index].displayed()).count().max(1) as f64;
    let params = Params {
        outer_width: width,
        outer_height: height,
        padding,
        available_width,
        available_height,
        v_box_max_width: available_width / 2.0 / visible_vertical,
        h_box_max_height: available_height / 2.0,
    };

    let mut max_padding = padding;
    let mp = Padding { top: min_padding, right: min_padding, bottom: min_padding, left: min_padding };
    update_max_padding(&mut max_padding, mp);
    let mut area = Area {
        max_padding,
        w: available_width,
        h: available_height,
        x: padding.left,
        y: padding.top,
        top: padding.top,
        left: padding.left,
        bottom: padding.bottom,
        right: padding.right,
    };

    // setLayoutDims (per wrap; keyed by index since the lists hold copies)
    let mut stacks: HashMap<String, Stack> = HashMap::new();
    for w in vertical.iter().chain(horizontal.iter()) {
        if let Some(s) = &w.stack {
            let st = stacks.entry(s.clone()).or_default();
            st.count += 1.0;
            st.weight += w.stack_weight;
        }
    }
    let mut dims: HashMap<usize, (f64, f64)> = HashMap::new();
    for w in vertical.iter().chain(horizontal.iter()) {
        let factor = w.stack.as_ref().and_then(|s| stacks.get(s)).map(|st| w.stack_weight / st.weight);
        let (lw, lh) = if w.horizontal {
            (
                factor.map_or(if w.full_size { params.available_width } else { 0.0 }, |f| {
                    f * params.v_box_max_width
                }),
                params.h_box_max_height,
            )
        } else {
            (
                params.v_box_max_width,
                factor.map_or(if w.full_size { params.available_height } else { 0.0 }, |f| {
                    f * params.h_box_max_height
                }),
            )
        };
        dims.insert(w.index, (lw, lh));
    }
    let mut sizes: HashMap<usize, f64> = HashMap::new();
    let mut lists = [full, vertical.clone(), horizontal];
    for l in lists.iter_mut() {
        for w in l.iter_mut() {
            let (lw, lh) = dims.get(&w.index).copied().unwrap_or((0.0, 0.0));
            w.width = lw;
            w.height = lh;
        }
    }
    let [full, vertical, horizontal] = lists;

    fit_boxes(items, &full, &mut area, &params, &mut stacks, &mut sizes, c);
    fit_boxes(items, &vertical, &mut area, &params, &mut stacks, &mut sizes, c);
    if fit_boxes(items, &horizontal, &mut area, &params, &mut stacks, &mut sizes, c) {
        fit_boxes(items, &vertical, &mut area, &params, &mut stacks, &mut sizes, c);
    }

    // handleMaxPadding
    let update_pos = |area: &mut Area, p: Position| {
        let change = (side(&area.max_padding, p) - *area.side(p)).max(0.0);
        *area.side(p) += change;
        change
    };
    let dy = update_pos(&mut area, Position::Top);
    area.y += dy;
    let dx = update_pos(&mut area, Position::Left);
    area.x += dx;
    update_pos(&mut area, Position::Right);
    update_pos(&mut area, Position::Bottom);

    for s in stacks.values_mut() {
        s.placed = 0.0;
        s.start = None;
    }
    place_boxes(items, &left_and_top, &mut area, &params, &mut stacks);
    area.x += area.w;
    area.y += area.h;
    place_boxes(items, &right_and_bottom, &mut area, &params, &mut stacks);

    ChartArea { left: area.left, top: area.top, right: area.left + area.w, bottom: area.top + area.h }
}

fn update_max_padding(max: &mut Padding, p: Padding) {
    max.top = max.top.max(p.top);
    max.left = max.left.max(p.left);
    max.bottom = max.bottom.max(p.bottom);
    max.right = max.right.max(p.right);
}

fn margins_for(horizontal: bool, area: &Area) -> Padding {
    let mut m = Padding::default();
    let set = |m: &mut Padding, p: Position| {
        let v = side(&area.max_padding, p).max(match p {
            Position::Top => area.top,
            Position::Left => area.left,
            Position::Bottom => area.bottom,
            Position::Right => area.right,
        });
        match p {
            Position::Top => m.top = v,
            Position::Left => m.left = v,
            Position::Bottom => m.bottom = v,
            Position::Right => m.right = v,
        }
    };
    if horizontal {
        set(&mut m, Position::Left);
        set(&mut m, Position::Right);
    } else {
        set(&mut m, Position::Top);
        set(&mut m, Position::Bottom);
    }
    m
}

#[allow(clippy::too_many_arguments)]
fn fit_boxes(
    items: &mut [&mut dyn LayoutItem],
    boxes: &[Wrap],
    area: &mut Area,
    params: &Params,
    stacks: &mut HashMap<String, Stack>,
    sizes: &mut HashMap<usize, f64>,
    c: &mut dyn Canvas,
) -> bool {
    let mut refit_boxes: Vec<Wrap> = Vec::new();
    let mut refit = false;
    let mut changed = false;
    for layout in boxes {
        let b = &mut items[layout.index];
        let w = if layout.width != 0.0 { layout.width } else { area.w };
        let h = if layout.height != 0.0 { layout.height } else { area.h };
        b.update(w, h, margins_for(layout.horizontal, area), c);

        // updateDims
        let prev = sizes.get(&layout.index).copied().unwrap_or(0.0);
        if prev != 0.0 {
            *area.side(layout.pos) -= prev;
        }
        let box_size = if layout.horizontal { b.height() } else { b.width() };
        let size = match layout.stack.as_ref().and_then(|s| stacks.get_mut(s)) {
            Some(st) => {
                st.size = st.size.max(box_size);
                st.size / st.count
            }
            None => box_size,
        };
        sizes.insert(layout.index, size);
        *area.side(layout.pos) += size;
        if let Some(p) = b.padding() {
            update_max_padding(&mut area.max_padding, p);
        }
        let new_w = (params.outer_width - combined(area, Position::Left, Position::Right)).max(0.0);
        let new_h = (params.outer_height - combined(area, Position::Top, Position::Bottom)).max(0.0);
        let w_changed = new_w != area.w;
        let h_changed = new_h != area.h;
        area.w = new_w;
        area.h = new_h;
        let (same, other) = if layout.horizontal { (w_changed, h_changed) } else { (h_changed, w_changed) };

        refit |= same && !refit_boxes.is_empty();
        changed = changed || other;
        if !b.full_size() {
            refit_boxes.push(layout.clone());
        }
    }
    (refit && fit_boxes(items, &refit_boxes, area, params, stacks, sizes, c)) || changed
}

fn combined(area: &Area, a: Position, b: Position) -> f64 {
    let get = |p: Position| {
        side(&area.max_padding, p).max(match p {
            Position::Top => area.top,
            Position::Left => area.left,
            Position::Bottom => area.bottom,
            Position::Right => area.right,
        })
    };
    get(a) + get(b)
}

fn place_boxes(
    items: &mut [&mut dyn LayoutItem],
    boxes: &[Wrap],
    area: &mut Area,
    params: &Params,
    stacks: &mut HashMap<String, Stack>,
) {
    let user = params.padding;
    let (mut x, mut y) = (area.x, area.y);
    for layout in boxes {
        let b = &mut items[layout.index];
        let mut fallback = Stack { count: 1.0, placed: 0.0, weight: 1.0, size: 0.0, start: None };
        let stack = match layout.stack.as_ref() {
            Some(s) => stacks.get_mut(s).unwrap_or(&mut fallback),
            None => &mut fallback,
        };
        let mut weight = layout.stack_weight / stack.weight;
        if !weight.is_finite() || weight == 0.0 {
            weight = 1.0;
        }
        if layout.horizontal {
            let width = area.w * weight;
            let height = if stack.size != 0.0 { stack.size } else { b.height() };
            if let Some(s) = stack.start {
                y = s;
            }
            if b.full_size() {
                b.set_box(user.left, y, params.outer_width - user.right - user.left, height);
            } else {
                b.set_box(area.left + stack.placed, y, width, height);
            }
            stack.start = Some(y);
            stack.placed += width;
            y = b.bottom();
        } else {
            let height = area.h * weight;
            let width = if stack.size != 0.0 { stack.size } else { b.width() };
            if let Some(s) = stack.start {
                x = s;
            }
            if b.full_size() {
                b.set_box(x, user.top, width, params.outer_height - user.bottom - user.top);
            } else {
                b.set_box(x, area.top + stack.placed, width, height);
            }
            stack.start = Some(x);
            stack.placed += height;
            x = b.right();
        }
    }
    area.x = x;
    area.y = y;
}

// Scales participate in layout directly.
impl LayoutItem for crate::scale::Scale {
    fn position(&self) -> Position {
        self.position
    }
    fn weight(&self) -> f64 {
        self.weight
    }
    fn full_size(&self) -> bool {
        self.full_size
    }
    fn is_horizontal(&self) -> bool {
        crate::scale::Scale::is_horizontal(self)
    }
    fn displayed(&self) -> bool {
        self.opts().get("display").and_then(serde_json::Value::as_bool) != Some(false)
    }
    fn stack(&self) -> Option<String> {
        self.opts().str("stack").map(str::to_owned)
    }
    fn stack_weight(&self) -> f64 {
        self.opts().num_or("stackWeight", 1.0)
    }
    fn update(&mut self, max_width: f64, max_height: f64, margins: Padding, c: &mut dyn Canvas) {
        crate::scale::Scale::update(self, max_width, max_height, margins, c);
    }
    fn padding(&self) -> Option<Padding> {
        Some(self.padding)
    }
    fn width(&self) -> f64 {
        self.width
    }
    fn height(&self) -> f64 {
        self.height
    }
    fn set_box(&mut self, left: f64, top: f64, width: f64, height: f64) {
        self.left = left;
        self.top = top;
        self.right = left + width;
        self.bottom = top + height;
        self.width = width;
        self.height = height;
    }
    fn right(&self) -> f64 {
        self.right
    }
    fn bottom(&self) -> f64 {
        self.bottom
    }
}
