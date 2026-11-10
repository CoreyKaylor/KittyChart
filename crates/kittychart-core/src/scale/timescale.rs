//! `time` and `timeseries` scales: a port of Chart.js `scale.time.js` and `scale.timeseries.js`
//! over the date-fns-compatible [`crate::time`] adapter.

use kittychart_canvas::Canvas;
use serde_json::Value;

use super::{Scale, ScaleKind, Tick};
use crate::config::Label;
use crate::math::{limit_value, to_radians};
use crate::time::{self, DATETIME_FORMAT, Unit};

#[derive(Debug, Clone, Default)]
pub(super) struct TimeState {
    unit: Option<Unit>,
    major_unit: Option<Unit>,
    offsets: (f64, f64, f64), // start, end, factor
    /// timeseries lookup table: (time, pos).
    table: Vec<(f64, f64)>,
    min_pos: f64,
    table_range: f64,
}

/// `_lookup(table, value)`: bracketing indices for a sorted array.
fn lookup(table: &[f64], value: f64) -> (usize, usize) {
    let (mut lo, mut hi) = (0, table.len().saturating_sub(1));
    while hi - lo > 1 {
        let mid = (lo + hi) >> 1;
        if table[mid] < value {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo, hi)
}

fn sort_unique(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite());
    v.sort_by(f64::total_cmp);
    v.dedup();
    v
}

impl Scale {
    fn time_opts(&self) -> crate::options::Scopes<'_> {
        self.opts().child("time")
    }

    fn min_unit(&self) -> Unit {
        self.time_opts().str("minUnit").and_then(Unit::parse).unwrap_or(Unit::Millisecond)
    }

    fn fixed_unit(&self) -> Option<Unit> {
        self.time_opts().str("unit").and_then(Unit::parse)
    }

    /// `displayFormats[unit]`, user formats merged over the adapter's.
    fn display_format(&self, unit: Unit) -> String {
        self.time_opts()
            .child("displayFormats")
            .str(unit.name())
            .map_or_else(|| unit.default_format().to_owned(), str::to_owned)
    }

    /// The time scale `parse()`: adapter parse, then optional `round`.
    pub(super) fn parse_time(&self, raw: &Value) -> Option<f64> {
        let v = time::parse(raw)?;
        Some(match self.time_opts().str("round").and_then(Unit::parse) {
            Some(u) => time::start_of(v, u),
            None => v,
        })
    }

    fn label_timestamps(&self) -> Vec<f64> {
        let ts: Vec<f64> = self
            .labels
            .iter()
            .filter_map(|l| match l {
                Label::Text(s) => self.parse_time(&Value::String(s.clone())),
                Label::Lines(_) => None,
            })
            .collect();
        sort_unique(ts)
    }

    pub(super) fn determine_time_limits(&mut self, ranges: &[(f64, f64)]) {
        let unit = self.fixed_unit().unwrap_or(Unit::Day);
        let (mut min, mut max, min_defined, max_defined) = self.user_bounds();
        let mut apply = |lo: f64, hi: f64| {
            if !min_defined && !lo.is_nan() {
                min = min.min(lo);
            }
            if !max_defined && !hi.is_nan() {
                max = max.max(hi);
            }
        };
        if !min_defined || !max_defined {
            let labels = self.label_timestamps();
            if let (Some(a), Some(b)) = (labels.first(), labels.last()) {
                apply(*a, *b);
            }
            let source_labels = self.ticks_opts().str("source") == Some("labels");
            if !self.bounds_is_ticks() || !source_labels {
                for &(a, b) in ranges {
                    apply(a, b);
                }
            }
        }
        // No data: Chart.js uses "now". KittyChart is deterministic, so it uses the epoch.
        let min = if min.is_finite() { min } else { time::start_of(0.0, unit) };
        let max = if max.is_finite() { max } else { time::end_of(0.0, unit) + 1.0 };
        self.min = min.min(max - 1.0);
        self.max = (min + 1.0).max(max);
    }

    fn label_capacity(&self, example: f64, c: &mut dyn Canvas) -> f64 {
        let fmt = self
            .fixed_unit()
            .map_or_else(|| self.display_format(Unit::Millisecond), |u| self.display_format(u));
        let label = time::format(example, &fmt);
        let t = self.ticks_opts();
        let font = self.tick_font();
        c.save();
        c.set_font(font.to_canvas());
        let width = f64::from(c.measure_text(&label).width);
        c.restore();
        let angle = to_radians(if self.is_horizontal() {
            t.num_or("maxRotation", 50.0)
        } else {
            t.num_or("minRotation", 0.0)
        });
        let (cos, sin) = (angle.cos(), angle.sin());
        let w = width * cos + font.size * sin;
        let h = width * sin + font.size * cos;
        let capacity = (if self.is_horizontal() { self.width / w } else { self.height / h }).floor() - 1.0;
        if capacity > 0.0 { capacity } else { 1.0 }
    }

    fn unit_for_auto_ticks(&self, min: f64, max: f64, capacity: f64) -> Unit {
        let start = Unit::ALL.iter().position(|u| *u == self.min_unit()).unwrap_or(0);
        for u in &Unit::ALL[start..Unit::ALL.len() - 1] {
            let (common, size, steps) = u.interval();
            let factor = steps.unwrap_or(9007199254740991.0);
            if common && ((max - min) / (factor * size)).ceil() <= capacity {
                return *u;
            }
        }
        Unit::Year
    }

    fn unit_for_formatting(&self, num_ticks: usize, min: f64, max: f64) -> Unit {
        let lo = Unit::ALL.iter().position(|u| *u == self.min_unit()).unwrap_or(0);
        for u in Unit::ALL[lo..].iter().rev() {
            if u.interval().0 && time::diff(max, min, *u) >= num_ticks as f64 - 1.0 {
                return *u;
            }
        }
        Unit::ALL[lo]
    }

    fn generate_time_ticks(&self, c: &mut dyn Canvas) -> Vec<f64> {
        let (min, max) = (self.min, self.max);
        if self.kind == ScaleKind::TimeSeries {
            let mut ts = self.data_timestamps.clone();
            if ts.is_empty() || !ts.contains(&min) {
                ts.insert(0, min);
            }
            if !ts.contains(&max) || ts.len() == 1 {
                ts.push(max);
            }
            ts.sort_by(f64::total_cmp);
            return ts;
        }
        let minor = self
            .fixed_unit()
            .unwrap_or_else(|| self.unit_for_auto_ticks(min, max, self.label_capacity(min, c)));
        let step = self.ticks_opts().num("stepSize").filter(|s| *s > 0.0).unwrap_or(1.0) as i64;
        let first = time::start_of(min, minor);
        if time::diff(max, min, minor) > 100000.0 * step as f64 {
            return vec![]; // Chart.js throws: "too far apart"
        }
        let data = (self.ticks_opts().str("source") == Some("data")).then(|| self.data_timestamps.clone());
        let mut ticks: Vec<f64> = Vec::new();
        let mut add_tick = |t: f64| match &data {
            None => ticks.push(t),
            Some(ts) if !ts.is_empty() => {
                let (lo, hi) = lookup(ts, t);
                ticks.push(if ts[lo] >= t { ts[lo] } else { ts[hi] });
            }
            Some(_) => {}
        };
        let (mut time, mut count) = (first, 0);
        while time < max {
            add_tick(time);
            time = time::add(time, step, minor);
            count += 1;
        }
        if time == max || self.bounds_is_ticks() || count == 1 {
            add_tick(time);
        }
        sort_unique(ticks)
    }

    /// `buildTicks` for time scales.
    pub(super) fn build_time_ticks(&mut self, c: &mut dyn Canvas) -> Vec<Tick> {
        let source_labels = self.ticks_opts().str("source") == Some("labels");
        let timestamps = if source_labels { self.label_timestamps() } else { self.generate_time_ticks(c) };
        if self.bounds_is_ticks() && !timestamps.is_empty() {
            self.min = self.user.min.filter(|v| *v != 0.0).unwrap_or(timestamps[0]);
            self.max = self.user.max.filter(|v| *v != 0.0).unwrap_or(timestamps[timestamps.len() - 1]);
        }
        let (min, max) = (self.min, self.max);
        let ticks: Vec<f64> = timestamps.iter().copied().filter(|t| *t >= min && *t <= max).collect();
        let auto_skip = self.ticks_opts().bool_or("autoSkip", true);
        let unit = self.fixed_unit().unwrap_or_else(|| {
            if auto_skip {
                self.unit_for_auto_ticks(min, max, self.label_capacity(min, c))
            } else {
                self.unit_for_formatting(ticks.len(), min, max)
            }
        });
        let major_enabled = self.ticks_opts().child("major").bool_or("enabled", false);
        let major_unit = if !major_enabled || unit == Unit::Year {
            None
        } else {
            let i = Unit::ALL.iter().position(|u| *u == unit).unwrap_or(0);
            Unit::ALL[i + 1..].iter().copied().find(|u| u.interval().0)
        };
        self.time.unit = Some(unit);
        self.time.major_unit = major_unit;
        self.init_time_offsets(&timestamps);
        let mut ticks = ticks;
        if self.reverse() {
            ticks.reverse();
        }
        let mut out: Vec<Tick> = ticks
            .iter()
            .map(|v| Tick { value: *v, label: Label::Text(String::new()), major: false, significand: None })
            .collect();
        // setMajorTicks
        if let (Some(mu), Some(first), Some(last)) =
            (major_unit, out.first().map(|t| t.value), out.last().map(|t| t.value))
        {
            let mut major = time::start_of(first, mu);
            while major <= last {
                if let Some(t) = out.iter_mut().find(|t| t.value == major) {
                    t.major = true;
                }
                major = time::add(major, 1, mu);
            }
        }
        out
    }

    fn init_time_offsets(&mut self, timestamps: &[f64]) {
        if self.kind == ScaleKind::TimeSeries {
            // buildLookupTable over data + label timestamps
            let data = self.data_timestamps.clone();
            let labels = self.label_timestamps();
            let all = match (data.is_empty(), labels.is_empty()) {
                (false, false) => sort_unique([data, labels].concat()),
                (false, true) => data,
                _ => labels,
            };
            let (min, max) = (self.min, self.max);
            let items: Vec<f64> = all.into_iter().filter(|t| *t >= min && *t <= max).collect();
            let table = if items.len() < 2 {
                vec![(min, 0.0), (max, 1.0)]
            } else {
                let n = items.len();
                (0..n)
                    .filter(|&i| {
                        let next = items.get(i + 1).copied().unwrap_or(f64::NAN);
                        let prev = if i > 0 { items[i - 1] } else { f64::NAN };
                        crate::math::js_round((next + prev) / 2.0) != items[i]
                    })
                    .map(|i| (items[i], i as f64 / (n - 1) as f64))
                    .collect()
            };
            self.time.table = table;
            self.time.min_pos = interpolate(&self.time.table, self.min);
            self.time.table_range = interpolate(&self.time.table, self.max) - self.time.min_pos;
        }
        let (mut start, mut end) = (0.0, 0.0);
        if self.offset() && !timestamps.is_empty() {
            let n = timestamps.len();
            let first = self.time_decimal(timestamps[0]);
            start = if n == 1 { 1.0 - first } else { (self.time_decimal(timestamps[1]) - first) / 2.0 };
            let last = self.time_decimal(timestamps[n - 1]);
            end = if n == 1 { last } else { (last - self.time_decimal(timestamps[n - 2])) / 2.0 };
        }
        let limit = if timestamps.len() < 3 { 0.5 } else { 0.25 };
        start = limit_value(start, 0.0, limit);
        end = limit_value(end, 0.0, limit);
        self.time.offsets = (start, end, 1.0 / (start + 1.0 + end));
    }

    fn time_decimal(&self, v: f64) -> f64 {
        if v.is_nan() {
            return f64::NAN;
        }
        match self.kind {
            ScaleKind::TimeSeries => {
                (interpolate(&self.time.table, v) - self.time.min_pos) / self.time.table_range
            }
            _ => (v - self.min) / (self.max - self.min),
        }
    }

    pub(super) fn time_pixel_for_value(&self, v: f64) -> f64 {
        let (start, _, factor) = self.time.offsets;
        let factor = if factor == 0.0 { 1.0 } else { factor };
        self.pixel_for_decimal((start + self.time_decimal(v)) * factor)
    }

    pub(super) fn time_value_for_pixel(&self, pixel: f64) -> f64 {
        let (_, end, factor) = self.time.offsets;
        let pos = self.decimal_for_pixel(pixel) / factor - end;
        self.min + pos * (self.max - self.min)
    }

    /// `_tickFormatFunction` with the default formats (major ticks use the major unit's).
    pub(super) fn time_tick_label(&self, tick: &Tick) -> String {
        let unit = self.time.unit.unwrap_or(Unit::Day);
        let fmt = match self.time.major_unit {
            Some(mu) if tick.major => self.display_format(mu),
            _ => self.display_format(unit),
        };
        time::format(tick.value, &fmt)
    }

    pub(super) fn time_label_for_value(&self, v: f64) -> String {
        let fmt = self.time_opts().str("tooltipFormat").map_or_else(
            || self.time_opts().child("displayFormats").str("datetime").unwrap_or(DATETIME_FORMAT).to_owned(),
            str::to_owned,
        );
        time::format(v, &fmt)
    }
}

/// timeseries `interpolate(table, val)` (time → pos).
fn interpolate(table: &[(f64, f64)], val: f64) -> f64 {
    if table.is_empty() {
        return 0.0;
    }
    let (mut lo, mut hi) = (0, table.len() - 1);
    if val >= table[lo].0 && val <= table[hi].0 {
        let times: Vec<f64> = table.iter().map(|t| t.0).collect();
        (lo, hi) = lookup(&times, val);
    }
    let (ps, pt) = table[lo];
    let (ns, nt) = table[hi];
    let span = ns - ps;
    if span != 0.0 { pt + (nt - pt) * (val - ps) / span } else { pt }
}
