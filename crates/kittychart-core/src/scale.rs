//! Cartesian scales: a port of Chart.js `core.scale.js`, `scale.linearbase.js`, `scale.linear.js`,
//! `scale.category.js` and `core.scale.autoskip.js`.

use kittychart_canvas::{Canvas, Color, Paint, TextAlign, TextBaseline};
use serde_json::Value;

mod timescale;

use crate::config::{Label, js_number_to_string};
use crate::draw::{TextOpts, clip_area, render_text};
use crate::format;
use crate::math::*;
use crate::options::{FontSpec, Padding, Scopes, as_num, to_dimension};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    X,
    Y,
}

impl Axis {
    pub fn as_str(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Top,
    Bottom,
    Left,
    Right,
}

impl Position {
    pub fn parse(s: &str) -> Option<Position> {
        match s {
            "top" => Some(Position::Top),
            "bottom" => Some(Position::Bottom),
            "left" => Some(Position::Left),
            "right" => Some(Position::Right),
            _ => None,
        }
    }
    pub fn is_horizontal(self) -> bool {
        matches!(self, Position::Top | Position::Bottom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleKind {
    Category,
    Linear,
    Logarithmic,
    Time,
    TimeSeries,
}

impl ScaleKind {
    pub fn is_time(self) -> bool {
        matches!(self, ScaleKind::Time | ScaleKind::TimeSeries)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub value: f64,
    pub label: Label,
    pub major: bool,
    /// Logarithmic ticks: the mantissa step (1–9, 10, 15) that generated this tick.
    pub significand: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Size {
    width: f64,
    height: f64,
}

#[derive(Debug, Clone, Default)]
struct LabelSizes {
    first: Size,
    last: Size,
    widest: Size,
    highest: Size,
    widths: Vec<f64>,
    heights: Vec<f64>,
}

/// User-facing min/max options after parsing.
#[derive(Debug, Clone, Copy, Default)]
struct UserBounds {
    min: Option<f64>,
    max: Option<f64>,
    suggested_min: Option<f64>,
    suggested_max: Option<f64>,
}

/// A cartesian scale and its layout box.
#[derive(Debug, Clone)]
pub struct Scale {
    pub id: String,
    pub kind: ScaleKind,
    pub axis: Axis,
    pub position: Position,
    /// Merged scale options (user config, type overrides and scale defaults).
    pub config: Value,
    /// Route targets: `{"color": chart color, "borderColor": chart borderColor}`.
    routes: Value,
    /// Chart-level font fallback (options.font over defaults.font).
    font_fallback: Value,

    // Box
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub width: f64,
    pub height: f64,
    pub max_width: f64,
    pub max_height: f64,
    margins: Padding,
    pub padding: Padding,
    pub weight: f64,
    pub full_size: bool,

    // Values
    pub min: f64,
    pub max: f64,
    range: Option<(f64, f64)>,
    user: UserBounds,
    pub ticks: Vec<Tick>,
    pub label_rotation: f64,
    label_sizes: std::cell::RefCell<Option<LabelSizes>>,
    /// Calls scriptable options (see [`crate::script`]).
    pub script: Option<crate::script::Script>,
    pub labels: Vec<Label>,
    /// Number of visible datasets bound to this scale (for `display: 'auto'`).
    pub visible_datasets: usize,
    /// A logarithmic scale saw a zero value (`_zero`).
    pub log_zero: bool,
    /// Time scales: every parsed data timestamp on this axis (sorted, unique), set by the chart.
    pub data_timestamps: Vec<f64>,
    time: timescale::TimeState,

    // Pixel mapping
    start_pixel: f64,
    end_pixel: f64,
    reverse_pixels: bool,
    pub length: f64,
    max_length: f64,
    start_value: f64,
    value_range: f64,
    border_value: f64,

    // Chart context
    pub chart_width: f64,
    pub chart_height: f64,
    pub dpr: f64,
}

impl Scale {
    /// Build a scale from merged options. `labels` are the category labels (`data.labels`).
    pub fn new(
        id: &str,
        kind: ScaleKind,
        axis: Axis,
        config: Value,
        routes: Value,
        font_fallback: Value,
        labels: Vec<Label>,
    ) -> Scale {
        let s = Scopes::new([&config]);
        let position = s.str("position").and_then(Position::parse).unwrap_or(if axis == Axis::X {
            Position::Bottom
        } else {
            Position::Left
        });
        let weight = s.num_or("weight", 0.0);
        let full_size = s.bool_or("fullSize", false);
        let labels = match s.get("labels").and_then(Value::as_array) {
            Some(l) => l.iter().map(Label::from_value).collect(),
            None => labels,
        };
        drop(s);
        let mut scale = Scale {
            id: id.to_owned(),
            kind,
            axis,
            position,
            config,
            routes,
            font_fallback,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            width: 0.0,
            height: 0.0,
            max_width: 0.0,
            max_height: 0.0,
            margins: Padding::default(),
            padding: Padding::default(),
            weight,
            full_size,
            min: 0.0,
            max: 0.0,
            range: None,
            user: UserBounds::default(),
            ticks: Vec::new(),
            label_rotation: 0.0,
            label_sizes: std::cell::RefCell::new(None),
            script: None,
            labels,
            visible_datasets: 0,
            log_zero: false,
            data_timestamps: vec![],
            time: timescale::TimeState::default(),
            start_pixel: 0.0,
            end_pixel: 0.0,
            reverse_pixels: false,
            length: 0.0,
            max_length: 0.0,
            start_value: 0.0,
            value_range: 0.0,
            border_value: 0.0,
            chart_width: 0.0,
            chart_height: 0.0,
            dpr: 1.0,
        };
        let s = Scopes::new([&scale.config]);
        let user = UserBounds {
            min: s.get("min").and_then(|v| scale.parse(v, None)),
            max: s.get("max").and_then(|v| scale.parse(v, None)),
            suggested_min: s.get("suggestedMin").and_then(|v| scale.parse(v, None)),
            suggested_max: s.get("suggestedMax").and_then(|v| scale.parse(v, None)),
        };
        drop(s);
        scale.user = user;
        scale
    }

    // ---------------------------------------------------------------- options

    pub fn opts(&self) -> Scopes<'_> {
        Scopes::new([&self.config]).with_script(self.script_ctx())
    }

    /// The scale context (`type: "scale"`); indexable options get the tick context.
    fn script_ctx(&self) -> Option<std::rc::Rc<crate::options::ScriptCtx<'_>>> {
        let script = self.script.clone()?;
        let base = self.scale_context();
        let per_index = Box::new(move |i: usize| self.tick_context(i));
        Some(std::rc::Rc::new(crate::options::ScriptCtx { script, base, per_index: Some(per_index) }))
    }

    fn scale_context(&self) -> Value {
        serde_json::json!({
            "type": "scale",
            "scale": { "id": self.id, "axis": self.axis.as_str(), "min": self.min, "max": self.max },
        })
    }

    fn tick_context(&self, i: usize) -> Value {
        match self.ticks.get(i) {
            Some(t) => serde_json::json!({
                "type": "tick",
                "index": i,
                "tick": { "value": t.value, "label": label_json(&t.label), "major": t.major },
            }),
            None => serde_json::json!({ "index": i }),
        }
    }

    /// `ticks.callback(value, index, ticks)`: replaces the default labels. A callback that
    /// returns undefined hides the tick's label.
    fn apply_tick_callback(&mut self) {
        let (Some(script), Some(id)) = (self.script.clone(), self.ticks_opts().fn_id("callback")) else {
            return;
        };
        let ticks: Vec<Value> = self
            .ticks
            .iter()
            .map(|t| serde_json::json!({ "value": t.value, "label": label_json(&t.label), "major": t.major }))
            .collect();
        let ticks = Value::Array(ticks);
        let labels: Vec<Label> = (0..self.ticks.len())
            .map(|i| {
                let mut ctx = self.tick_context(i);
                ctx["scale"] = self.scale_context()["scale"].clone();
                ctx["__args"] = serde_json::json!([self.ticks[i].value, i, ticks]);
                script.call(&id, &ctx).map_or(Label::Text(String::new()), |v| Label::from_value(&v))
            })
            .collect();
        for (t, l) in self.ticks.iter_mut().zip(labels) {
            t.label = l;
        }
    }
    fn ticks_opts(&self) -> Scopes<'_> {
        self.opts().child("ticks").then([&self.routes["ticks"]])
    }
    fn grid_opts(&self) -> Scopes<'_> {
        self.opts().child("grid").then([&self.routes["grid"]])
    }
    fn border_opts(&self) -> Scopes<'_> {
        self.opts().child("border").then([&self.routes["border"]])
    }
    fn title_opts(&self) -> Scopes<'_> {
        self.opts().child("title").then([&self.routes["title"]])
    }
    fn font_of(&self, s: &Scopes<'_>) -> FontSpec {
        FontSpec::resolve(&s.child("font"), &Scopes::new([&self.font_fallback]))
    }
    fn tick_font(&self) -> FontSpec {
        self.font_of(&self.ticks_opts())
    }

    pub fn is_horizontal(&self) -> bool {
        self.position.is_horizontal() || self.axis == Axis::X
    }
    pub fn reverse(&self) -> bool {
        self.opts().bool_or("reverse", false)
    }
    pub fn offset(&self) -> bool {
        self.opts().bool_or("offset", false)
    }
    pub fn begin_at_zero(&self) -> bool {
        self.opts().bool_or("beginAtZero", false)
    }
    fn bounds_is_ticks(&self) -> bool {
        self.opts().str("bounds").unwrap_or("ticks") == "ticks"
    }
    /// `stacked`: `None` (unset), `Some(false)`, `Some(true)`, or "single" as `Some(true)` + flag.
    pub fn stacked(&self) -> Option<Stacked> {
        match self.opts().get("stacked") {
            Some(Value::Bool(true)) => Some(Stacked::Yes),
            Some(Value::Bool(false)) => Some(Stacked::No),
            Some(Value::String(s)) if s == "single" => Some(Stacked::Single),
            _ => None,
        }
    }

    pub fn is_visible(&self) -> bool {
        match self.opts().get("display") {
            Some(Value::String(s)) if s == "auto" => self.visible_datasets > 0,
            Some(Value::Bool(b)) => *b,
            _ => true,
        }
    }

    // ---------------------------------------------------------------- data

    /// Parse a raw data value for this scale. Category scales map labels to indices
    /// (adding unknown string labels, as Chart.js does). Linear scales coerce numbers.
    pub fn parse(&self, raw: &Value, index: Option<usize>) -> Option<f64> {
        match self.kind {
            ScaleKind::Linear => match raw {
                Value::Null => None,
                v => as_num(v).filter(|n| n.is_finite()),
            },
            // Zero and negatives can't be plotted on a log axis; zero is noted via `note_zero`.
            ScaleKind::Logarithmic => as_num(raw).filter(|n| n.is_finite() && *n > 0.0),
            ScaleKind::Time | ScaleKind::TimeSeries => self.parse_time(raw),
            ScaleKind::Category => {
                if raw.is_null() {
                    return None;
                }
                if let Some(i) = index
                    && self.labels.get(i).is_some_and(|l| label_matches(l, raw))
                {
                    return Some(i as f64);
                }
                let first = self.labels.iter().position(|l| label_matches(l, raw));
                let last = self.labels.iter().rposition(|l| label_matches(l, raw));
                let idx = match (first, last) {
                    (Some(f), Some(l)) if f == l => Some(f as f64),
                    (Some(_), Some(_)) => index.map(|i| i as f64),
                    // findOrAddLabel → addIfString: an unknown value keeps the data index
                    // (or the raw number when there is no index).
                    _ => index.map(|i| i as f64).or_else(|| as_num(raw).filter(|n| n.is_finite())),
                };
                let max = self.labels.len().saturating_sub(1) as f64;
                idx.map(|i| limit_value(js_round(i), 0.0, max))
            }
        }
    }

    /// Add a string label that wasn't in `labels` (category `findOrAddLabel`). Returns its index.
    pub fn add_label_if_missing(&mut self, raw: &Value) -> Option<usize> {
        if self.kind != ScaleKind::Category {
            return None;
        }
        let Value::String(s) = raw else { return None };
        if self.labels.iter().any(|l| label_matches(l, raw)) {
            return None;
        }
        self.labels.push(Label::Text(s.clone()));
        Some(self.labels.len() - 1)
    }

    /// `getUserBounds()`: (min, max, minDefined, maxDefined) with suggested values folded in.
    pub fn user_bounds(&self) -> (f64, f64, bool, bool) {
        let u = self.user;
        let user_min = u.min.unwrap_or(f64::INFINITY);
        let user_max = u.max.unwrap_or(f64::NEG_INFINITY);
        let sug_min = u.suggested_min.unwrap_or(f64::INFINITY);
        let sug_max = u.suggested_max.unwrap_or(f64::NEG_INFINITY);
        let min = if user_min.is_finite() { user_min } else { sug_min };
        let max = if user_max.is_finite() { user_max } else { sug_max };
        (min, max, user_min.is_finite(), user_max.is_finite())
    }

    /// `getMinMax`: combine dataset ranges (already stacked where needed) with user bounds.
    pub fn min_max(&self, dataset_ranges: &[(f64, f64)]) -> (f64, f64) {
        let (mut min, mut max, min_defined, max_defined) = self.user_bounds();
        if min_defined && max_defined {
            return (min, max);
        }
        for &(rmin, rmax) in dataset_ranges {
            if !min_defined {
                min = min.min(rmin);
            }
            if !max_defined {
                max = max.max(rmax);
            }
        }
        if max_defined && min > max {
            min = max;
        }
        if min_defined && min > max {
            max = min;
        }
        let fin = |v: f64, alt: f64| if v.is_finite() { v } else { alt };
        (fin(min, fin(max, min)), fin(max, fin(min, max)))
    }

    /// Logarithmic `parse`: a zero value is remembered (it plots at the scale minimum).
    pub fn note_zero(&mut self, raw: &Value) {
        if self.kind == ScaleKind::Logarithmic && as_num(raw) == Some(0.0) {
            self.log_zero = true;
        }
    }

    /// `determineDataLimits` + `_addGrace`, given dataset ranges for this scale.
    pub fn determine_data_limits(&mut self, dataset_ranges: &[(f64, f64)]) {
        let (min, max) = self.min_max(dataset_ranges);
        match self.kind {
            ScaleKind::Category => {
                let (_, _, min_defined, max_defined) = self.user_bounds();
                let (mut min, mut max) = (min, max);
                if self.bounds_is_ticks() {
                    if !min_defined {
                        min = 0.0;
                    }
                    if !max_defined {
                        max = self.labels.len() as f64 - 1.0;
                    }
                }
                self.min = min;
                self.max = max;
            }
            ScaleKind::Linear => {
                self.min = if min.is_finite() { min } else { 0.0 };
                self.max = if max.is_finite() { max } else { 1.0 };
                self.handle_tick_range_options();
            }
            ScaleKind::Logarithmic => {
                self.determine_log_limits(min, max);
                self.range = None;
                return; // logarithmic scales don't apply grace
            }
            ScaleKind::Time | ScaleKind::TimeSeries => {
                self.determine_time_limits(dataset_ranges);
                self.range = None;
                return;
            }
        }
        // _addGrace
        let grace = self.opts().get("grace").cloned();
        let change = to_dimension(grace.as_ref(), (self.max - self.min) / 2.0);
        let begin_at_zero = self.begin_at_zero();
        let keep_zero = |v: f64, add: f64| if begin_at_zero && v == 0.0 { 0.0 } else { v + add };
        self.range = Some((keep_zero(self.min, -change.abs()), keep_zero(self.max, change)));
    }

    fn determine_log_limits(&mut self, min: f64, max: f64) {
        let change_exp = |v: f64, m: f64| 10f64.powf(v.log10().floor() + m);
        // JS `null` for "no data"; `null <= 0` is true in the comparisons below.
        let mut lmin: Option<f64> = min.is_finite().then(|| min.max(0.0));
        let mut lmax: Option<f64> = max.is_finite().then(|| max.max(0.0));
        if self.begin_at_zero() {
            self.log_zero = true;
        }
        let user_min_finite = self.user.min.is_some_and(f64::is_finite);
        if self.log_zero
            && lmin != self.user.suggested_min
            && !user_min_finite
            && let Some(m) = lmin
        {
            lmin = Some(if min == change_exp(m, 0.0) { change_exp(m, -1.0) } else { change_exp(m, 0.0) });
        }
        let (_, _, min_defined, max_defined) = self.user_bounds();
        let le0 = |v: Option<f64>| v.is_none_or(|v| v <= 0.0);
        if lmin == lmax {
            if le0(lmin) {
                if !min_defined {
                    lmin = Some(1.0);
                }
                if !max_defined {
                    lmax = Some(10.0);
                }
            } else {
                let (a, b) = (lmin.unwrap_or(1.0), lmax.unwrap_or(10.0));
                if !min_defined {
                    lmin = Some(change_exp(a, -1.0));
                }
                if !max_defined {
                    lmax = Some(change_exp(b, 1.0));
                }
            }
        }
        if le0(lmin) && !min_defined {
            lmin = lmax.map(|m| change_exp(m, -1.0));
        }
        if le0(lmax) && !max_defined {
            lmax = lmin.map(|m| change_exp(m, 1.0));
        }
        self.min = lmin.unwrap_or(1.0);
        self.max = lmax.unwrap_or(10.0);
    }

    fn handle_tick_range_options(&mut self) {
        let (_, _, min_defined, max_defined) = self.user_bounds();
        let (mut min, mut max) = (self.min, self.max);
        let begin_at_zero = self.begin_at_zero();
        if begin_at_zero {
            let (smin, smax) = (sign(min), sign(max));
            if smin < 0.0 && smax < 0.0 {
                if !max_defined {
                    max = 0.0;
                }
            } else if smin > 0.0 && smax > 0.0 && !min_defined {
                min = 0.0;
            }
        }
        if min == max {
            let offset = if max == 0.0 { 1.0 } else { (max * 0.05).abs() };
            if !max_defined {
                max += offset;
            }
            if !begin_at_zero && !min_defined {
                min -= offset;
            }
        }
        self.min = min;
        self.max = max;
    }

    // ---------------------------------------------------------------- update (layout)

    /// `Scale.update(maxWidth, maxHeight, margins)`.
    pub fn update(&mut self, max_width: f64, max_height: f64, margins: Padding, c: &mut dyn Canvas) {
        self.max_width = max_width;
        self.max_height = max_height;
        self.margins = margins;
        self.ticks.clear();
        self.label_sizes.replace(None);

        // setDimensions
        if self.is_horizontal() {
            self.width = max_width;
            self.left = 0.0;
            self.right = self.width;
        } else {
            self.height = max_height;
            self.top = 0.0;
            self.bottom = self.height;
        }
        self.padding = Padding::default();
        self.max_length = if self.is_horizontal() {
            self.width + margins.left + margins.right
        } else {
            self.height + margins.top + margins.bottom
        };

        self.ticks = if self.kind.is_time() { self.build_time_ticks(c) } else { self.build_ticks() };
        self.generate_tick_labels();
        self.apply_tick_callback();
        self.configure();
        self.calculate_label_rotation(c);
        let t = self.ticks_opts();
        let skip =
            t.bool_or("display", true) && (t.bool_or("autoSkip", true) || t.str("source") == Some("auto"));
        drop(t);
        if skip {
            self.ticks = self.auto_skip(c);
            self.label_sizes.replace(None);
        }
        self.fit(c);
    }

    fn build_ticks(&mut self) -> Vec<Tick> {
        match self.kind {
            ScaleKind::Category => {
                let offset = self.offset();
                let (min, max) = (self.min, self.max);
                let count = if max >= min { (max - min) as usize + 1 } else { 0 };
                self.value_range = ((count as f64) - if offset { 0.0 } else { 1.0 }).max(1.0);
                self.start_value = min - if offset { 0.5 } else { 0.0 };
                let mut ticks = Vec::with_capacity(count);
                let mut v = min;
                while v <= max {
                    ticks.push(Tick {
                        value: v,
                        label: Label::Text(String::new()),
                        major: false,
                        significand: None,
                    });
                    v += 1.0;
                }
                ticks
            }
            ScaleKind::Linear => self.build_linear_ticks(),
            ScaleKind::Time | ScaleKind::TimeSeries => unreachable!("time scales use build_time_ticks"),
            ScaleKind::Logarithmic => {
                let mut ticks = generate_log_ticks(self.user.min, self.user.max, self.min, self.max);
                if self.bounds_is_ticks() {
                    for t in &ticks {
                        if !t.value.is_nan() {
                            self.min = self.min.min(t.value);
                            self.max = self.max.max(t.value);
                        }
                    }
                }
                if self.reverse() {
                    ticks.reverse();
                }
                ticks
            }
        }
    }

    fn build_linear_ticks(&mut self) -> Vec<Tick> {
        let t = self.ticks_opts();
        let max_ticks = self.tick_limit().max(2.0);
        let (min_opt, max_opt) = (self.user.min, self.user.max);
        let tg = TickGen {
            max_ticks,
            bounds_ticks: self.bounds_is_ticks(),
            min: min_opt,
            max: max_opt,
            precision: t.num("precision"),
            step: t.num("stepSize").filter(|s| *s > 0.0),
            count: t.num("count"),
            max_digits: self.max_digits(),
            horizontal: self.is_horizontal(),
            min_rotation: t.num_or("minRotation", 0.0),
            include_bounds: t.get("includeBounds").and_then(Value::as_bool) != Some(false),
        };
        let (rmin, rmax) = self.range.unwrap_or((self.min, self.max));
        let values = generate_ticks(&tg, rmin, rmax);
        drop(t);
        if tg.bounds_ticks {
            for v in &values {
                if !v.is_nan() {
                    self.min = self.min.min(*v);
                    self.max = self.max.max(*v);
                }
            }
        }
        let mut ticks: Vec<Tick> = values
            .into_iter()
            .map(|value| Tick { value, label: Label::Text(String::new()), major: false, significand: None })
            .collect();
        if self.reverse() {
            ticks.reverse();
        }
        ticks
    }

    fn tick_limit(&self) -> f64 {
        let t = self.ticks_opts();
        let mut max_ticks_limit = t.num("maxTicksLimit");
        let max_ticks = if let Some(step) = t.num("stepSize").filter(|s| *s > 0.0) {
            ((self.max / step).ceil() - (self.min / step).floor() + 1.0).min(1000.0)
        } else {
            max_ticks_limit = Some(max_ticks_limit.filter(|v| *v != 0.0).unwrap_or(11.0));
            self.compute_tick_limit()
        };
        match max_ticks_limit {
            Some(l) => l.min(max_ticks),
            None => max_ticks,
        }
    }

    fn compute_tick_limit(&self) -> f64 {
        let horizontal = self.is_horizontal();
        let length = if horizontal { self.width } else { self.height };
        let min_rotation = to_radians(self.ticks_opts().num_or("minRotation", 0.0));
        let mut ratio = if horizontal { min_rotation.sin() } else { min_rotation.cos() };
        if ratio == 0.0 {
            ratio = 0.001;
        }
        let lh = self.tick_font().line_height;
        (length / 40f64.min(lh / ratio)).ceil()
    }

    fn max_digits(&self) -> f64 {
        let lh = self.tick_font().line_height;
        (if self.is_horizontal() { self.width } else { self.height }) / lh
    }

    /// `generateTickLabels` with the default callbacks, then drop ticks with no label.
    fn generate_tick_labels(&mut self) {
        if self.kind.is_time() {
            let labels: Vec<String> = self.ticks.iter().map(|t| self.time_tick_label(t)).collect();
            for (t, l) in self.ticks.iter_mut().zip(labels) {
                t.label = Label::Text(l);
            }
            return;
        }
        let values: Vec<f64> = self.ticks.iter().map(|t| t.value).collect();
        for (i, tick) in self.ticks.iter_mut().enumerate() {
            tick.label = match self.kind {
                ScaleKind::Category => {
                    let v = tick.value;
                    if v >= 0.0 && (v as usize) < self.labels.len() {
                        self.labels[v as usize].clone()
                    } else {
                        Label::Text(js_number_to_string(v))
                    }
                }
                ScaleKind::Linear => Label::Text(format::numeric_tick(tick.value, i, &values)),
                ScaleKind::Time | ScaleKind::TimeSeries => unreachable!("handled above"),
                ScaleKind::Logarithmic => {
                    // Ticks.formatters.logarithmic: label only "round" mantissas, plus the top 20%.
                    let v = tick.value;
                    let remain = tick
                        .significand
                        .filter(|s| *s != 0.0)
                        .unwrap_or_else(|| v / 10f64.powf(v.log10().floor()));
                    let label = if v == 0.0 {
                        "0".to_owned()
                    } else if [1.0, 2.0, 3.0, 5.0, 10.0, 15.0].contains(&remain)
                        || (i as f64) > 0.8 * values.len() as f64
                    {
                        format::numeric_tick(v, i, &values)
                    } else {
                        String::new()
                    };
                    Label::Text(label)
                }
            };
        }
    }

    /// `getLabelForValue` (used by legends and tooltips).
    pub fn label_for_value(&self, value: f64) -> String {
        match self.kind {
            ScaleKind::Category => {
                if value >= 0.0 && (value as usize) < self.labels.len() {
                    self.labels[value as usize].lines().join(",")
                } else {
                    js_number_to_string(value)
                }
            }
            ScaleKind::Linear | ScaleKind::Logarithmic => format::format_default(value),
            ScaleKind::Time | ScaleKind::TimeSeries => self.time_label_for_value(value),
        }
    }

    /// `configure()`: pixel range and value mapping for the current box.
    pub fn configure(&mut self) {
        let mut reverse_pixels = self.reverse();
        let (start_pixel, end_pixel) = if self.is_horizontal() {
            (self.left, self.right)
        } else {
            reverse_pixels = !reverse_pixels;
            (self.top, self.bottom)
        };
        self.start_pixel = start_pixel;
        self.end_pixel = end_pixel;
        self.reverse_pixels = reverse_pixels;
        self.length = end_pixel - start_pixel;

        match self.kind {
            ScaleKind::Category => {
                if !self.is_horizontal() {
                    self.reverse_pixels = !self.reverse_pixels;
                }
                // _startValue and _valueRange were set in build_ticks.
            }
            ScaleKind::Linear => {
                let (mut start, mut end) = (self.min, self.max);
                if self.offset() && !self.ticks.is_empty() {
                    let off = (end - start) / ((self.ticks.len() as f64 - 1.0).max(1.0)) / 2.0;
                    start -= off;
                    end += off;
                }
                self.start_value = start;
                self.value_range = end - start;
            }
            ScaleKind::Logarithmic => {
                self.start_value = self.min.log10();
                self.value_range = self.max.log10() - self.min.log10();
            }
            ScaleKind::Time | ScaleKind::TimeSeries => {}
        }
    }

    fn calculate_label_rotation(&mut self, c: &mut dyn Canvas) {
        self.label_rotation = self.compute_label_rotation(c);
    }

    fn compute_label_rotation(&self, c: &mut dyn Canvas) -> f64 {
        let t = self.ticks_opts();
        let num_ticks = ticks_limit(self.ticks.len(), t.num("maxTicksLimit"));
        let min_rotation = t.num_or("minRotation", 0.0);
        let max_rotation = t.num_or("maxRotation", 50.0);
        let mut label_rotation = min_rotation;
        if !self.is_visible()
            || !t.bool_or("display", true)
            || min_rotation >= max_rotation
            || num_ticks <= 1
            || !self.is_horizontal()
        {
            return min_rotation;
        }
        let tick_padding = t.num_or("padding", 3.0);
        let sizes = self.get_label_sizes(c);
        let max_label_width = sizes.widest.width;
        let max_label_height = sizes.highest.height;
        let max_width = limit_value(self.chart_width - max_label_width, 0.0, self.max_width);
        let offset = self.offset();
        let n = num_ticks as f64;
        let mut tick_width = if offset { self.max_width / n } else { max_width / (n - 1.0) };
        if max_label_width + 6.0 > tick_width {
            tick_width = max_width / (n - if offset { 0.5 } else { 1.0 });
            let max_height =
                self.max_height - tick_mark_length(&self.grid_opts()) - tick_padding - self.title_height();
            let diag = (max_label_width * max_label_width + max_label_height * max_label_height).sqrt();
            label_rotation =
                to_degrees((limit_value((sizes.highest.height + 6.0) / tick_width, -1.0, 1.0)).asin().min(
                    limit_value(max_height / diag, -1.0, 1.0).asin()
                        - limit_value(max_label_height / diag, -1.0, 1.0).asin(),
                ));
            label_rotation = min_rotation.max(max_rotation.min(label_rotation));
        }
        label_rotation
    }

    fn title_height(&self) -> f64 {
        let t = self.title_opts();
        if !t.bool_or("display", false) {
            return 0.0;
        }
        let font = self.font_of(&t);
        let padding = Padding::from_value(t.get("padding"));
        let lines = t.get("text").and_then(Value::as_array).map_or(1, Vec::len) as f64;
        lines * font.line_height + padding.height()
    }

    fn get_label_sizes(&self, c: &mut dyn Canvas) -> LabelSizes {
        if let Some(s) = self.label_sizes.borrow().as_ref() {
            return s.clone();
        }
        let t = self.ticks_opts();
        let limit = ticks_limit(self.ticks.len(), t.num("maxTicksLimit"));
        let font = self.tick_font();
        let sizes = compute_label_sizes(&self.ticks, limit, &font, c);
        self.label_sizes.replace(Some(sizes.clone()));
        sizes
    }

    /// `fit()`: the box's size along the cross axis, and edge padding for overhanging labels.
    fn fit(&mut self, c: &mut dyn Canvas) {
        let sizes =
            if self.is_visible() && !self.ticks.is_empty() { Some(self.get_label_sizes(c)) } else { None };
        let mut min_w = 0.0;
        let mut min_h = 0.0;
        let horizontal = self.is_horizontal();
        let mut pending_padding = None;
        if self.is_visible() {
            let title_height = self.title_height();
            let tick_len = tick_mark_length(&self.grid_opts());
            if horizontal {
                min_w = self.max_width;
                min_h = tick_len + title_height;
            } else {
                min_h = self.max_height;
                min_w = tick_len + title_height;
            }
            let t = self.ticks_opts();
            if t.bool_or("display", true) && !self.ticks.is_empty() {
                let mirror = t.bool_or("mirror", false);
                let tick_padding = t.num_or("padding", 3.0) * 2.0;
                let sizes = sizes.clone().unwrap_or_default();
                let rad = to_radians(self.label_rotation);
                let (cos, sin) = (rad.cos(), rad.sin());
                if horizontal {
                    let lh = if mirror { 0.0 } else { sin * sizes.widest.width + cos * sizes.highest.height };
                    min_h = self.max_height.min(min_h + lh + tick_padding);
                } else {
                    let lw = if mirror { 0.0 } else { cos * sizes.widest.width + sin * sizes.highest.height };
                    min_w = self.max_width.min(min_w + lw + tick_padding);
                }
                pending_padding = Some((sizes.first, sizes.last, sin, cos));
            }
        }
        if let Some((f, l, sin, cos)) = pending_padding {
            self.calculate_padding(f, l, sin, cos);
        }
        // _handleMargins
        self.margins.left = self.padding.left.max(self.margins.left);
        self.margins.top = self.padding.top.max(self.margins.top);
        self.margins.right = self.padding.right.max(self.margins.right);
        self.margins.bottom = self.padding.bottom.max(self.margins.bottom);

        if horizontal {
            self.width = self.chart_width - self.margins.left - self.margins.right;
            self.length = self.width;
            self.height = min_h;
        } else {
            self.width = min_w;
            self.height = self.chart_height - self.margins.top - self.margins.bottom;
            self.length = self.height;
        }
    }

    fn calculate_padding(&mut self, first: Size, last: Size, sin: f64, cos: f64) {
        let t = self.ticks_opts();
        let align = t.str("align").unwrap_or("center").to_owned();
        let align = align.as_str();
        let padding = t.num_or("padding", 3.0);
        drop(t);
        let is_rotated = self.label_rotation != 0.0;
        let labels_below_ticks = self.position != Position::Top && self.axis == Axis::X;
        if self.is_horizontal() {
            let offset_left = self.pixel_for_tick(0).unwrap_or(self.left) - self.left;
            let offset_right =
                self.right - self.pixel_for_tick(self.ticks.len().saturating_sub(1)).unwrap_or(self.right);
            let (mut pl, mut pr) = (0.0, 0.0);
            if is_rotated {
                if labels_below_ticks {
                    pl = cos * first.width;
                    pr = sin * last.height;
                } else {
                    pl = sin * first.height;
                    pr = cos * last.width;
                }
            } else if align == "start" {
                pr = last.width;
            } else if align == "end" {
                pl = first.width;
            } else if align != "inner" {
                pl = first.width / 2.0;
                pr = last.width / 2.0;
            }
            self.padding.left =
                ((pl - offset_left + padding) * self.width / (self.width - offset_left)).max(0.0);
            self.padding.right =
                ((pr - offset_right + padding) * self.width / (self.width - offset_right)).max(0.0);
        } else {
            let (mut pt, mut pb) = (last.height / 2.0, first.height / 2.0);
            if align == "start" {
                pt = 0.0;
                pb = first.height;
            } else if align == "end" {
                pt = last.height;
                pb = 0.0;
            }
            self.padding.top = pt + padding;
            self.padding.bottom = pb + padding;
        }
    }

    fn tick_size(&self, c: &mut dyn Canvas) -> f64 {
        let pad = self.ticks_opts().num_or("autoSkipPadding", 0.0);
        let rot = to_radians(self.label_rotation);
        let (cos, sin) = (rot.cos().abs(), rot.sin().abs());
        let horizontal = self.is_horizontal();
        let sizes = self.get_label_sizes(c);
        let w = sizes.widest.width + pad;
        let h = sizes.highest.height + pad;
        if horizontal {
            if h * cos > w * sin { w / cos } else { h / sin }
        } else if h * sin < w * cos {
            h / cos
        } else {
            w / sin
        }
    }

    /// `autoSkip` (no major ticks for category/linear scales).
    fn auto_skip(&self, c: &mut dyn Canvas) -> Vec<Tick> {
        let offset = self.offset();
        let tick_len = self.tick_size(c);
        let max_scale = self.length / tick_len + if offset { 0.0 } else { 1.0 };
        let max_chart = self.max_length / tick_len;
        let determined = max_scale.min(max_chart).floor();
        let limit = match self.ticks_opts().num("maxTicksLimit").filter(|v| *v != 0.0) {
            Some(l) => l.min(determined),
            None => determined,
        };
        auto_skip_ticks(&self.ticks, limit, self.ticks_opts().child("major").bool_or("enabled", false))
    }

    // ---------------------------------------------------------------- value <-> pixel

    pub fn pixel_for_decimal(&self, decimal: f64) -> f64 {
        let d = if self.reverse_pixels { 1.0 - decimal } else { decimal };
        let pixel = self.start_pixel + d * self.length;
        let aligned = if self.opts().bool_or("alignToPixels", false) {
            align_pixel(self.dpr, pixel, 0.0)
        } else {
            pixel
        };
        int16_range(aligned)
    }

    pub fn decimal_for_pixel(&self, pixel: f64) -> f64 {
        let d = (pixel - self.start_pixel) / self.length;
        if self.reverse_pixels { 1.0 - d } else { d }
    }

    pub fn pixel_for_value(&self, value: f64) -> f64 {
        if self.kind.is_time() {
            return self.time_pixel_for_value(value);
        }
        if self.kind == ScaleKind::Logarithmic {
            // Zero (and values below the axis) plot at the minimum.
            let v = if value == 0.0 { self.min } else { value };
            if v.is_nan() {
                return f64::NAN;
            }
            let d = if v == self.min { 0.0 } else { (v.log10() - self.start_value) / self.value_range };
            return self.pixel_for_decimal(d);
        }
        if value.is_nan() {
            return f64::NAN;
        }
        self.pixel_for_decimal((value - self.start_value) / self.value_range)
    }

    pub fn value_for_pixel(&self, pixel: f64) -> f64 {
        let v = self.start_value + self.decimal_for_pixel(pixel) * self.value_range;
        match self.kind {
            ScaleKind::Category => js_round(v),
            ScaleKind::Logarithmic => 10f64.powf(v),
            ScaleKind::Linear => v,
            ScaleKind::Time | ScaleKind::TimeSeries => self.time_value_for_pixel(pixel),
        }
    }

    pub fn pixel_for_tick(&self, index: usize) -> Option<f64> {
        self.ticks.get(index).map(|t| self.pixel_for_value(t.value))
    }

    pub fn base_value(&self) -> f64 {
        let (min, max) = (self.min, self.max);
        if min < 0.0 && max < 0.0 {
            max
        } else if min > 0.0 && max > 0.0 {
            min
        } else {
            0.0
        }
    }

    pub fn base_pixel(&self) -> f64 {
        match self.kind {
            ScaleKind::Category => self.bottom,
            ScaleKind::Linear | ScaleKind::Logarithmic | ScaleKind::Time | ScaleKind::TimeSeries => {
                self.pixel_for_value(self.base_value())
            }
        }
    }

    pub fn start_pixel(&self) -> f64 {
        self.start_pixel
    }
    pub fn end_pixel(&self) -> f64 {
        self.end_pixel
    }

    /// `getLineWidthForValue`: the grid line width at a tick value, or 0.
    pub fn line_width_for_value(&self, value: f64) -> f64 {
        let g = self.grid_opts();
        if !self.is_visible() || !g.bool_or("display", true) {
            return 0.0;
        }
        match self.ticks.iter().position(|t| t.value == value) {
            Some(i) => g.num_at("lineWidth", Some(i)).unwrap_or(1.0),
            None => 0.0,
        }
    }

    // ---------------------------------------------------------------- drawing

    fn grid_pixel(&self, index: usize, offset_grid: bool) -> Option<f64> {
        let n = self.ticks.len();
        let valid = index.min(n.saturating_sub(1));
        let mut line = self.pixel_for_tick(valid)?;
        if offset_grid {
            let off = if n == 1 {
                (line - self.start_pixel).max(self.end_pixel - line)
            } else if index == 0 {
                (self.pixel_for_tick(1)? - line) / 2.0
            } else {
                (line - self.pixel_for_tick(valid - 1)?) / 2.0
            };
            line += if valid < index { off } else { -off };
            let eps = 1e-6;
            if line < self.start_pixel.min(self.end_pixel) - eps
                || line > self.start_pixel.max(self.end_pixel) + eps
            {
                return None;
            }
        }
        Some(line)
    }

    /// Draw the parts of the scale that sit below the datasets: background, grid, border,
    /// title, tick labels (Chart.js layers with z ≤ 0).
    pub fn draw(&mut self, c: &mut dyn Canvas, area: &ChartArea) {
        if !self.is_visible() {
            return;
        }
        if let Some(bg) = self.opts().color_at("backgroundColor", None) {
            c.save();
            c.set_fill_style(Paint::Solid(bg));
            c.fill_rect(self.left as f32, self.top as f32, self.width as f32, self.height as f32);
            c.restore();
        }
        self.border_value = self.draw_grid(c, area);
        self.draw_border(c);
        self.draw_title(c, area);
        self.draw_labels(c, area);
    }

    fn draw_grid(&self, c: &mut dyn Canvas, area: &ChartArea) -> f64 {
        let g = self.grid_opts();
        let b = self.border_opts();
        let offset = g.bool_or("offset", false);
        let horizontal = self.is_horizontal();
        let ticks_length = self.ticks.len() + usize::from(offset);
        let tl = tick_mark_length(&g);
        let axis_width = if b.bool_or("display", true) { b.num_or("width", 1.0) } else { 0.0 };
        let half = axis_width / 2.0;
        let dpr = self.dpr;
        let align_border = |p: f64| align_pixel(dpr, p, axis_width);

        let (mut tx1, mut ty1, mut tx2, mut ty2) = (0.0, 0.0, 0.0, 0.0);
        let (mut x1, mut y1, mut x2, mut y2) = (0.0, 0.0, 0.0, 0.0);
        let border_value;
        match self.position {
            Position::Top => {
                border_value = align_border(self.bottom);
                ty1 = self.bottom - tl;
                ty2 = border_value - half;
                y1 = align_border(area.top) + half;
                y2 = area.bottom;
            }
            Position::Bottom => {
                border_value = align_border(self.top);
                y1 = area.top;
                y2 = align_border(area.bottom) - half;
                ty1 = border_value + half;
                ty2 = self.top + tl;
            }
            Position::Left => {
                border_value = align_border(self.right);
                tx1 = self.right - tl;
                tx2 = border_value - half;
                x1 = align_border(area.left) + half;
                x2 = area.right;
            }
            Position::Right => {
                border_value = align_border(self.left);
                x1 = area.left;
                x2 = align_border(area.right) - half;
                tx1 = border_value + half;
                tx2 = self.left + tl;
            }
        }
        if !g.bool_or("display", true) {
            return border_value;
        }
        let limit = self.ticks_opts().num("maxTicksLimit").map_or(ticks_length, |l| l as usize).max(1);
        let step = ticks_length.div_ceil(limit).max(1);
        let draw_on_area = g.bool_or("drawOnChartArea", true);
        let draw_ticks = g.bool_or("drawTicks", true);
        let border_dash: Vec<f32> = b.num_list("dash").iter().map(|v| *v as f32).collect();
        let border_dash_offset = b.num_or("dashOffset", 0.0) as f32;
        let mut i = 0;
        while i < ticks_length {
            let line_width = g.num_at("lineWidth", Some(i)).unwrap_or(1.0);
            let color = g.color_at("color", Some(i));
            let tick_width = g.num_at("tickWidth", Some(i)).unwrap_or(line_width);
            let tick_color = g.color_at("tickColor", Some(i)).or(color);
            let tick_dash: Vec<f32> = g.num_list("tickBorderDash").iter().map(|v| *v as f32).collect();
            let tick_dash_offset = g.num_or("tickBorderDashOffset", 0.0) as f32;
            if let Some(line_value) = self.grid_pixel(i, offset) {
                let aligned = align_pixel(dpr, line_value, line_width);
                if horizontal {
                    tx1 = aligned;
                    tx2 = aligned;
                    x1 = aligned;
                    x2 = aligned;
                } else {
                    ty1 = aligned;
                    ty2 = aligned;
                    y1 = aligned;
                    y2 = aligned;
                }
                if draw_on_area {
                    draw_line(c, (x1, y1), (x2, y2), line_width, color, &border_dash, border_dash_offset);
                }
                if draw_ticks {
                    draw_line(
                        c,
                        (tx1, ty1),
                        (tx2, ty2),
                        tick_width,
                        tick_color,
                        &tick_dash,
                        tick_dash_offset,
                    );
                }
            }
            i += step;
        }
        border_value
    }

    fn draw_border(&self, c: &mut dyn Canvas) {
        let b = self.border_opts();
        let g = self.grid_opts();
        let axis_width = if b.bool_or("display", true) { b.num_or("width", 1.0) } else { 0.0 };
        if axis_width == 0.0 {
            return;
        }
        let last_line_width = g.num_at("lineWidth", Some(0)).unwrap_or(1.0);
        let (x1, x2, y1, y2);
        if self.is_horizontal() {
            x1 = align_pixel(self.dpr, self.left, axis_width) - axis_width / 2.0;
            x2 = align_pixel(self.dpr, self.right, last_line_width) + last_line_width / 2.0;
            y1 = self.border_value;
            y2 = self.border_value;
        } else {
            y1 = align_pixel(self.dpr, self.top, axis_width) - axis_width / 2.0;
            y2 = align_pixel(self.dpr, self.bottom, last_line_width) + last_line_width / 2.0;
            x1 = self.border_value;
            x2 = self.border_value;
        }
        let color = b.color_at("color", None);
        draw_line(c, (x1, y1), (x2, y2), axis_width, color, &[], 0.0);
    }

    fn draw_title(&self, c: &mut dyn Canvas, area: &ChartArea) {
        let t = self.title_opts();
        if !t.bool_or("display", false) {
            return;
        }
        let font = self.font_of(&t);
        let padding = Padding::from_value(t.get("padding"));
        let align = t.str("align").unwrap_or("center");
        let text = t.get("text").map(Label::from_value).unwrap_or(Label::Text(String::new()));
        let mut offset = font.line_height / 2.0;
        if self.position == Position::Bottom {
            offset += padding.bottom;
            offset += font.line_height * (text.line_count().saturating_sub(1)) as f64;
        } else {
            offset += padding.top;
        }
        let _ = area;
        let (title_x, title_y, max_width, rotation);
        if self.is_horizontal() {
            title_x = align_start_end(align, self.left, self.right);
            title_y = offset_from_edge(self, self.position, offset);
            max_width = self.right - self.left;
            rotation = 0.0;
        } else {
            title_x = offset_from_edge(self, self.position, offset);
            title_y = align_start_end(align, self.bottom, self.top);
            max_width = 0.0;
            rotation = if self.position == Position::Left {
                -std::f64::consts::FRAC_PI_2
            } else {
                std::f64::consts::FRAC_PI_2
            };
        }
        let reverse = self.reverse();
        let mut text_align = to_left_right_center(align);
        if (reverse && self.position != Position::Right) || (!reverse && self.position == Position::Right) {
            text_align = match text_align {
                TextAlign::Left => TextAlign::Right,
                TextAlign::Right => TextAlign::Left,
                a => a,
            };
        }
        render_text(
            c,
            &text,
            0.0,
            0.0,
            &font,
            &TextOpts {
                color: t.color_at("color", None),
                max_width: if self.is_horizontal() { Some(max_width) } else { None },
                rotation: Some(rotation),
                align: Some(text_align),
                baseline: Some(TextBaseline::Middle),
                translation: Some((title_x, title_y)),
                ..TextOpts::default()
            },
        );
    }

    fn draw_labels(&self, c: &mut dyn Canvas, _area: &ChartArea) {
        let t = self.ticks_opts();
        if !t.bool_or("display", true) {
            return;
        }
        let mirror = t.bool_or("mirror", false);
        let clip = if mirror {
            None
        } else if self.is_horizontal() {
            Some((0.0, self.top, self.chart_width, self.bottom))
        } else {
            Some((self.left, 0.0, self.right, self.chart_height))
        };
        if let Some((l, tp, r, b)) = clip {
            clip_area(c, l, tp, r, b);
        }
        let items = self.compute_label_items(c);
        for item in items {
            render_text(c, &item.label, 0.0, item.text_offset, &item.font, &item.opts);
        }
        if clip.is_some() {
            c.restore();
        }
    }

    fn compute_label_items(&self, c: &mut dyn Canvas) -> Vec<LabelItem> {
        let t = self.ticks_opts();
        let align = t.str("align").unwrap_or("center").to_owned();
        let cross_align = t.str("crossAlign").unwrap_or("near").to_owned();
        let padding = t.num_or("padding", 3.0);
        let mirror = t.bool_or("mirror", false);
        let label_offset = t.num_or("labelOffset", 0.0);
        let stroke_width = t.num_or("textStrokeWidth", 0.0);
        let stroke_color = t.color_at("textStrokeColor", None);
        let show_backdrop = t.bool_or("showLabelBackdrop", false);
        let backdrop_color = t.color_at("backdropColor", None);
        let backdrop_padding = Padding::from_value(t.get("backdropPadding"));
        let tl = tick_mark_length(&self.grid_opts());
        let tick_and_padding = tl + padding;
        let h_tick_and_padding = if mirror { -padding } else { tick_and_padding };
        let rotation = -to_radians(self.label_rotation);
        let font = self.tick_font();
        let horizontal = self.is_horizontal();
        let sizes = self.get_label_sizes(c);
        let mut baseline = TextBaseline::Middle;

        let (mut x, mut y) = (0.0, 0.0);
        let text_align: &str;
        match self.position {
            Position::Top => {
                y = self.bottom - h_tick_and_padding;
                text_align = self.x_axis_label_alignment(&align);
            }
            Position::Bottom => {
                y = self.top + h_tick_and_padding;
                text_align = self.x_axis_label_alignment(&align);
            }
            Position::Left | Position::Right => {
                let (ta, xx) =
                    self.y_axis_label_alignment(tl, &cross_align, mirror, padding, sizes.widest.width);
                text_align = ta;
                x = xx;
            }
        }
        if self.axis == Axis::Y {
            if align == "start" {
                baseline = TextBaseline::Top;
            } else if align == "end" {
                baseline = TextBaseline::Bottom;
            }
        }

        let n = self.ticks.len();
        let mut items = Vec::with_capacity(n);
        for i in 0..n {
            let label = self.ticks[i].label.clone();
            let pixel = self.pixel_for_tick(i).unwrap_or(0.0) + label_offset;
            let lh = font.line_height;
            let line_count = label.line_count() as f64;
            let half_count = line_count / 2.0;
            let mut tick_align = text_align;
            let text_offset;
            if horizontal {
                x = pixel;
                if text_align == "inner" {
                    tick_align = if i == n - 1 {
                        if self.reverse() { "left" } else { "right" }
                    } else if i == 0 {
                        if self.reverse() { "right" } else { "left" }
                    } else {
                        "center"
                    };
                }
                let mut off = if self.position == Position::Top {
                    if cross_align == "near" || rotation != 0.0 {
                        -line_count * lh + lh / 2.0
                    } else if cross_align == "center" {
                        -sizes.highest.height / 2.0 - half_count * lh + lh
                    } else {
                        -sizes.highest.height + lh / 2.0
                    }
                } else if cross_align == "near" || rotation != 0.0 {
                    lh / 2.0
                } else if cross_align == "center" {
                    sizes.highest.height / 2.0 - half_count * lh
                } else {
                    sizes.highest.height - line_count * lh
                };
                if mirror {
                    off *= -1.0;
                }
                if rotation != 0.0 && !show_backdrop {
                    x += (lh / 2.0) * rotation.sin();
                }
                text_offset = off;
            } else {
                y = pixel;
                text_offset = (1.0 - line_count) * lh / 2.0;
            }

            let backdrop = if show_backdrop {
                let h = sizes.heights.get(i).copied().unwrap_or(0.0);
                let w = sizes.widths.get(i).copied().unwrap_or(0.0);
                let mut top = text_offset - backdrop_padding.top;
                let mut left = -backdrop_padding.left;
                match baseline {
                    TextBaseline::Middle => top -= h / 2.0,
                    TextBaseline::Bottom => top -= h,
                    _ => {}
                }
                match text_align {
                    "center" => left -= w / 2.0,
                    "right" => left -= w,
                    "inner" => {
                        if i == n - 1 {
                            left -= w;
                        } else if i > 0 {
                            left -= w / 2.0;
                        }
                    }
                    _ => {}
                }
                backdrop_color
                    .map(|col| (left, top, w + backdrop_padding.width(), h + backdrop_padding.height(), col))
            } else {
                None
            };

            items.push(LabelItem {
                label,
                font: font.clone(),
                text_offset,
                opts: TextOpts {
                    rotation: Some(rotation),
                    color: t.color_at("color", Some(i)),
                    stroke_color,
                    stroke_width,
                    align: Some(parse_text_align(tick_align)),
                    baseline: Some(baseline),
                    translation: Some((x, y)),
                    backdrop,
                    ..TextOpts::default()
                },
            });
        }
        items
    }

    fn x_axis_label_alignment(&self, align: &str) -> &'static str {
        if self.label_rotation != 0.0 {
            return if self.position == Position::Top { "left" } else { "right" };
        }
        match align {
            "start" => "left",
            "end" => "right",
            "inner" => "inner",
            _ => "center",
        }
    }

    fn y_axis_label_alignment(
        &self,
        tl: f64,
        cross: &str,
        mirror: bool,
        padding: f64,
        widest: f64,
    ) -> (&'static str, f64) {
        let tick_and_padding = tl + padding;
        match self.position {
            Position::Left => {
                if mirror {
                    let x = self.right + padding;
                    match cross {
                        "near" => ("left", x),
                        "center" => ("center", x + widest / 2.0),
                        _ => ("right", x + widest),
                    }
                } else {
                    let x = self.right - tick_and_padding;
                    match cross {
                        "near" => ("right", x),
                        "center" => ("center", x - widest / 2.0),
                        _ => ("left", self.left),
                    }
                }
            }
            Position::Right => {
                if mirror {
                    let x = self.left + padding;
                    match cross {
                        "near" => ("right", x),
                        "center" => ("center", x - widest / 2.0),
                        _ => ("left", x - widest),
                    }
                } else {
                    let x = self.left + tick_and_padding;
                    match cross {
                        "near" => ("left", x),
                        "center" => ("center", x + widest / 2.0),
                        _ => ("right", self.right),
                    }
                }
            }
            _ => ("right", 0.0),
        }
    }
}

/// `stacked` scale option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stacked {
    No,
    Yes,
    Single,
}

/// The plot area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ChartArea {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl ChartArea {
    pub fn width(&self) -> f64 {
        self.right - self.left
    }
    pub fn height(&self) -> f64 {
        self.bottom - self.top
    }
}

struct LabelItem {
    label: Label,
    font: FontSpec,
    text_offset: f64,
    opts: TextOpts,
}

fn draw_line(
    c: &mut dyn Canvas,
    p1: (f64, f64),
    p2: (f64, f64),
    width: f64,
    color: Option<Color>,
    dash: &[f32],
    dash_offset: f32,
) {
    let Some(color) = color else { return };
    if width == 0.0 {
        return;
    }
    c.save();
    c.set_line_width(width as f32);
    c.set_stroke_style(Paint::Solid(color));
    c.set_line_dash(dash);
    c.set_line_dash_offset(dash_offset);
    c.begin_path();
    c.move_to(p1.0 as f32, p1.1 as f32);
    c.line_to(p2.0 as f32, p2.1 as f32);
    c.stroke();
    c.restore();
}

fn tick_mark_length(grid: &Scopes<'_>) -> f64 {
    if grid.bool_or("drawTicks", true) { grid.num_or("tickLength", 8.0) } else { 0.0 }
}

fn ticks_limit(len: usize, max_ticks_limit: Option<f64>) -> usize {
    match max_ticks_limit.filter(|v| *v > 0.0) {
        Some(l) => (l as usize).min(len),
        None => len,
    }
}

fn compute_label_sizes(ticks: &[Tick], limit: usize, font: &FontSpec, c: &mut dyn Canvas) -> LabelSizes {
    let length = ticks.len();
    c.save();
    c.set_font(font.to_canvas());
    let increment = length.checked_div(limit).unwrap_or(1).max(1);
    let mut widths = Vec::new();
    let mut heights = Vec::new();
    let mut i = 0;
    while i < length {
        let label = &ticks[i].label;
        let mut width: f64 = 0.0;
        let mut height = 0.0;
        for line in label.lines() {
            width = width.max(f64::from(c.measure_text(line).width));
            height += font.line_height;
        }
        widths.push(width);
        heights.push(height);
        i += increment;
    }
    c.restore();
    let widest_v = widths.iter().copied().fold(0.0, f64::max);
    let highest_v = heights.iter().copied().fold(0.0, f64::max);
    let widest = widths.iter().position(|w| *w == widest_v);
    let highest = heights.iter().position(|h| *h == highest_v);
    let at = |idx: Option<usize>| match idx {
        Some(i) => Size {
            width: widths.get(i).copied().unwrap_or(0.0),
            height: heights.get(i).copied().unwrap_or(0.0),
        },
        None => Size::default(),
    };
    // Chart.js indexes `last` as `length - 1` into the sampled arrays.
    let last_idx = length.checked_sub(1);
    LabelSizes {
        first: at(Some(0)),
        last: at(last_idx),
        widest: at(widest),
        highest: at(highest),
        widths: widths.clone(),
        heights: heights.clone(),
    }
}

/// `autoSkip` from `core.scale.autoskip.js`, given the tick limit.
fn auto_skip_ticks(ticks: &[Tick], limit: f64, major_enabled: bool) -> Vec<Tick> {
    let majors: Vec<usize> = if major_enabled {
        ticks.iter().enumerate().filter(|(_, t)| t.major).map(|(i, _)| i).collect()
    } else {
        vec![]
    };
    let n_major = majors.len();
    let mut out = Vec::new();
    if n_major as f64 > limit {
        // skipMajors
        let spacing = (n_major as f64 / limit).ceil() as usize;
        let mut count = 0;
        let mut next = majors.first().copied();
        for (i, t) in ticks.iter().enumerate() {
            if Some(i) == next {
                out.push(t.clone());
                count += 1;
                next = majors.get(count * spacing).copied();
            }
        }
        return out;
    }
    // calculateSpacing
    let spacing = {
        let base = ticks.len() as f64 / limit;
        match even_spacing(&majors) {
            Some(even) => {
                let factors = factorize(even as u64);
                factors[..factors.len().saturating_sub(1)]
                    .iter()
                    .map(|f| *f as f64)
                    .find(|f| *f > base)
                    .unwrap_or_else(|| base.max(1.0))
            }
            None => base.max(1.0),
        }
    };
    if n_major > 0 {
        let first = majors[0] as f64;
        let last = majors[n_major - 1] as f64;
        let avg = (n_major > 1).then(|| js_round((last - first) / (n_major as f64 - 1.0)));
        skip(ticks, &mut out, spacing, Some(avg.map_or(0.0, |a| first - a)), Some(first));
        for w in majors.windows(2) {
            skip(ticks, &mut out, spacing, Some(w[0] as f64), Some(w[1] as f64));
        }
        skip(ticks, &mut out, spacing, Some(last), Some(avg.map_or(ticks.len() as f64, |a| last + a)));
        return out;
    }
    skip(ticks, &mut out, spacing, None, None);
    out
}

fn even_spacing(arr: &[usize]) -> Option<usize> {
    if arr.len() < 2 {
        return None;
    }
    let diff = arr[0];
    arr.windows(2).all(|w| w[1] - w[0] == diff).then_some(diff)
}

/// `skip()` from autoskip.
fn skip(ticks: &[Tick], out: &mut Vec<Tick>, spacing: f64, major_start: Option<f64>, major_end: Option<f64>) {
    let start = major_start.unwrap_or(0.0);
    let end = major_end.unwrap_or(ticks.len() as f64).min(ticks.len() as f64);
    let mut spacing = spacing.ceil();
    // JS: `if (majorEnd)` - a zero end is falsy.
    if let Some(me) = major_end.filter(|v| *v != 0.0) {
        let length = me - start;
        spacing = length / (length / spacing).floor();
    }
    let mut count = 0.0;
    let mut next = start;
    while next < 0.0 {
        count += 1.0;
        next = js_round(start + count * spacing);
    }
    let mut i = start.max(0.0);
    while i < end {
        if i == next {
            out.push(ticks[i as usize].clone());
            count += 1.0;
            next = js_round(start + count * spacing);
        }
        i += 1.0;
    }
}

/// `generateTicks` from `scale.logarithmic.js`.
fn generate_log_ticks(user_min: Option<f64>, user_max: Option<f64>, min: f64, max: f64) -> Vec<Tick> {
    let log10_floor = |v: f64| v.log10().floor();
    let is_major = |v: f64| v / 10f64.powf(log10_floor(v)) == 1.0;
    let min = user_min.filter(|v| v.is_finite()).unwrap_or(min);
    let steps = |min: f64, max: f64, range_exp: f64| {
        let step = 10f64.powf(range_exp);
        (max / step).ceil() - (min / step).floor()
    };
    let start_exp = {
        let mut range_exp = log10_floor(max - min);
        while steps(min, max, range_exp) > 10.0 {
            range_exp += 1.0;
        }
        while steps(min, max, range_exp) < 10.0 {
            range_exp -= 1.0;
        }
        range_exp.min(log10_floor(min))
    };
    let mut ticks = Vec::new();
    let min_exp = log10_floor(min);
    let mut exp = start_exp;
    let mut precision = if exp < 0.0 { 10f64.powf(exp.abs()) } else { 1.0 };
    let step_size = 10f64.powf(exp);
    let base = if min_exp > exp { 10f64.powf(min_exp) } else { 0.0 };
    let start = js_round((min - base) * precision) / precision;
    let offset = ((min - base) / step_size / 10.0).floor() * step_size * 10.0;
    let mut significand = ((start - offset) / 10f64.powf(exp)).floor();
    let mut value = user_min
        .filter(|v| v.is_finite())
        .unwrap_or_else(|| js_round((base + offset + significand * 10f64.powf(exp)) * precision) / precision);
    let mut guard = 0;
    while value < max && guard < 10_000 {
        guard += 1;
        ticks.push(Tick {
            value,
            label: Label::Text(String::new()),
            major: is_major(value),
            significand: Some(significand),
        });
        if significand >= 10.0 {
            significand = if significand < 15.0 { 15.0 } else { 20.0 };
        } else {
            significand += 1.0;
        }
        if significand >= 20.0 {
            exp += 1.0;
            significand = 2.0;
            precision = if exp >= 0.0 { 1.0 } else { precision };
        }
        value = js_round((base + offset + significand * 10f64.powf(exp)) * precision) / precision;
    }
    let last = user_max.filter(|v| v.is_finite()).unwrap_or(value);
    ticks.push(Tick {
        value: last,
        label: Label::Text(String::new()),
        major: is_major(last),
        significand: Some(significand),
    });
    ticks
}

fn align_start_end(align: &str, start: f64, end: f64) -> f64 {
    match align {
        "start" => start,
        "end" => end,
        _ => (start + end) / 2.0,
    }
}

fn to_left_right_center(align: &str) -> TextAlign {
    match align {
        "start" => TextAlign::Left,
        "end" => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

fn parse_text_align(a: &str) -> TextAlign {
    match a {
        "left" => TextAlign::Left,
        "right" => TextAlign::Right,
        "start" => TextAlign::Start,
        "end" => TextAlign::End,
        _ => TextAlign::Center,
    }
}

fn offset_from_edge(s: &Scale, edge: Position, offset: f64) -> f64 {
    match edge {
        Position::Top => s.top + offset,
        Position::Left => s.left + offset,
        Position::Bottom => s.bottom - offset,
        Position::Right => s.right - offset,
    }
}

/// `labels.indexOf(raw)`: strict equality, so only string labels can match string data.
fn label_matches(l: &Label, raw: &Value) -> bool {
    matches!((l, raw), (Label::Text(t), Value::String(s)) if t == s)
}

// -------------------------------------------------------------------- numeric tick generation

/// Inputs to [`generate_ticks`] (Chart.js `generationOptions`).
#[derive(Debug, Clone)]
pub struct TickGen {
    pub max_ticks: f64,
    pub bounds_ticks: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub precision: Option<f64>,
    pub step: Option<f64>,
    pub count: Option<f64>,
    pub max_digits: f64,
    pub horizontal: bool,
    pub min_rotation: f64,
    pub include_bounds: bool,
}

/// `generateTicks` from `scale.linearbase.js`, line for line.
pub fn generate_ticks(o: &TickGen, rmin: f64, rmax: f64) -> Vec<f64> {
    const MIN_SPACING: f64 = 1e-14;
    let mut ticks = Vec::new();
    let unit = o.step.unwrap_or(1.0);
    let max_spaces = o.max_ticks - 1.0;
    let min_defined = o.min.is_some();
    let max_defined = o.max.is_some();
    let min_spacing = (rmax - rmin) / (o.max_digits + 1.0);
    let mut spacing = nice_num((rmax - rmin) / max_spaces / unit) * unit;

    if spacing < MIN_SPACING && !min_defined && !max_defined {
        return vec![rmin, rmax];
    }
    let mut num_spaces = (rmax / spacing).ceil() - (rmin / spacing).floor();
    if num_spaces > max_spaces {
        spacing = nice_num(num_spaces * spacing / max_spaces / unit) * unit;
    }
    if let Some(p) = o.precision {
        let factor = 10f64.powf(p);
        spacing = (spacing * factor).ceil() / factor;
    }
    let (mut nice_min, mut nice_max) = if o.bounds_ticks {
        ((rmin / spacing).floor() * spacing, (rmax / spacing).ceil() * spacing)
    } else {
        (rmin, rmax)
    };

    if let (Some(min), Some(max), Some(step)) = (o.min, o.max, o.step)
        && almost_whole((max - min) / step, spacing / 1000.0)
    {
        num_spaces = js_round(((max - min) / spacing).min(o.max_ticks));
        spacing = (max - min) / num_spaces;
        nice_min = min;
        nice_max = max;
    } else if let Some(count) = o.count {
        nice_min = o.min.unwrap_or(nice_min);
        nice_max = o.max.unwrap_or(nice_max);
        num_spaces = count - 1.0;
        spacing = (nice_max - nice_min) / num_spaces;
    } else {
        num_spaces = (nice_max - nice_min) / spacing;
        if almost_equals(num_spaces, js_round(num_spaces), spacing / 1000.0) {
            num_spaces = js_round(num_spaces);
        } else {
            num_spaces = num_spaces.ceil();
        }
    }

    let decimal_places = decimal_places(spacing).unwrap_or(0).max(decimal_places(nice_min).unwrap_or(0));
    let factor = 10f64.powf(o.precision.unwrap_or(f64::from(decimal_places)));
    nice_min = js_round(nice_min * factor) / factor;
    nice_max = js_round(nice_max * factor) / factor;

    let mut j = 0.0;
    if let Some(min) = o.min {
        if o.include_bounds && nice_min != min {
            ticks.push(min);
            if nice_min < min {
                j += 1.0;
            }
            if almost_equals(
                js_round((nice_min + j * spacing) * factor) / factor,
                min,
                relative_label_size(min, min_spacing, o),
            ) {
                j += 1.0;
            }
        } else if nice_min < min {
            j += 1.0;
        }
    }
    while j < num_spaces {
        let v = js_round((nice_min + j * spacing) * factor) / factor;
        if let Some(max) = o.max
            && v > max
        {
            break;
        }
        ticks.push(v);
        j += 1.0;
    }
    if let Some(max) = o.max
        && o.include_bounds
        && nice_max != max
    {
        if let Some(last) = ticks.last_mut()
            && almost_equals(*last, max, relative_label_size(max, min_spacing, o))
        {
            *last = max;
        } else {
            ticks.push(max);
        }
    } else if !max_defined || Some(nice_max) == o.max {
        ticks.push(nice_max);
    }
    ticks
}

pub(crate) fn relative_label_size(value: f64, min_spacing: f64, o: &TickGen) -> f64 {
    let rad = to_radians(o.min_rotation);
    let mut ratio = if o.horizontal { rad.sin() } else { rad.cos() };
    if ratio == 0.0 {
        ratio = 0.001;
    }
    let length = 0.75 * min_spacing * js_number_to_string(value).len() as f64;
    (min_spacing / ratio).min(length)
}

fn label_json(l: &Label) -> Value {
    match l {
        Label::Text(s) => Value::String(s.clone()),
        Label::Lines(v) => Value::Array(v.iter().cloned().map(Value::String).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tg(max_ticks: f64) -> TickGen {
        TickGen {
            max_ticks,
            bounds_ticks: true,
            min: None,
            max: None,
            precision: None,
            step: None,
            count: None,
            max_digits: 20.0,
            horizontal: false,
            min_rotation: 0.0,
            include_bounds: true,
        }
    }

    #[test]
    fn nice_ticks_for_common_ranges() {
        assert_eq!(
            generate_ticks(&tg(11.0), 0.0, 100.0),
            [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0]
        );
        assert_eq!(generate_ticks(&tg(6.0), 3.0, 97.0), [0.0, 20.0, 40.0, 60.0, 80.0, 100.0]);
        assert_eq!(
            generate_ticks(&tg(11.0), -0.37, 0.52),
            [-0.4, -0.3, -0.2, -0.1, 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6]
        );
    }

    #[test]
    fn user_bounds_and_step() {
        let mut g = tg(11.0);
        g.min = Some(5.0);
        g.max = Some(95.0);
        assert_eq!(
            generate_ticks(&g, 5.0, 95.0),
            [5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 95.0]
        );
        // With stepSize, Chart.js getTickLimit gives ceil(max/step) - floor(min/step) + 1 = 5.
        let mut g = tg(5.0);
        g.step = Some(25.0);
        g.min = Some(0.0);
        g.max = Some(100.0);
        assert_eq!(generate_ticks(&g, 0.0, 100.0), [0.0, 25.0, 50.0, 75.0, 100.0]);
    }
}
