//! The chart: config → scales, dataset metas and plugins → layout → elements → drawing.
//! Mirrors the flow of Chart.js `core.controller.js` `update()` + `draw()`.

use std::collections::HashMap;

use kittychart_canvas::{Canvas, Color, LineCap, LineJoin, Paint, TextAlign};
use serde_json::{Value, json};

use crate::config::{ChartConfig, ChartType, Label};
use crate::defaults::{BACKGROUND_COLORS, BORDER_COLORS, DEFAULTS, overrides_for};
use crate::draw::{PointStyle, clip_area};
use crate::element::{Bar, BarOptions, Edges, Line, LineOptions, Point, PointOptions, SpanGaps, Stepped};
use crate::layout::{self, LayoutItem};
use crate::math::{is_between, sign};
use crate::options::{Corners, FontSpec, Padding, Scopes, as_num, merge_if};
use crate::plugins::{Legend, LegendItem, LegendLabels, LegendTitle, TextBlock};
use crate::scale::{Axis, ChartArea, Position, Scale, ScaleKind, Stacked};

/// One parsed data point. Values are in scale units (category indices for category scales).
#[derive(Debug, Clone, Copy, Default)]
pub struct Parsed {
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Floating bar `[start, end]`.
    pub custom: Option<FloatBar>,
}

#[derive(Debug, Clone, Copy)]
pub struct FloatBar {
    pub bar_start: f64,
    pub bar_end: f64,
    pub start: f64,
    pub end: f64,
    pub min: f64,
    pub max: f64,
}

impl Parsed {
    fn get(&self, axis: Axis) -> Option<f64> {
        match axis {
            Axis::X => self.x,
            Axis::Y => self.y,
        }
    }
}

/// Per-dataset state (Chart.js `meta`).
#[derive(Debug, Clone)]
pub struct DatasetMeta {
    pub index: usize,
    pub kind: ChartType,
    pub order: f64,
    pub visible: bool,
    pub label: Label,
    pub index_axis: Axis,
    pub x_scale: usize,
    pub y_scale: usize,
    pub stack: Option<String>,
    pub stacked: Option<Stacked>,
    pub parsed: Vec<Parsed>,
    pub points: Vec<Point>,
    pub line: Option<Line>,
    pub bars: Vec<Bar>,
    pub clip: Option<Padding>,
}

impl DatasetMeta {
    fn i_scale(&self) -> usize {
        if self.index_axis == Axis::X { self.x_scale } else { self.y_scale }
    }
    fn v_scale(&self) -> usize {
        if self.index_axis == Axis::X { self.y_scale } else { self.x_scale }
    }
}

type StackKey = (String, u64);

/// A configured chart, ready to lay out and draw at any size.
#[derive(Debug)]
pub struct Chart {
    pub config: ChartConfig,
    /// Datasets after the `colors` plugin ran.
    datasets: Vec<Value>,
    pub scales: Vec<Scale>,
    pub metas: Vec<DatasetMeta>,
    stacks: HashMap<StackKey, HashMap<usize, Option<f64>>>,
    pub title: TextBlock,
    pub subtitle: TextBlock,
    pub legend: Legend,
    pub chart_area: ChartArea,
    pub width: f64,
    pub height: f64,
    pub dpr: f64,
    font_fallback: Value,
    /// Terminal-derived defaults (see [`crate::theme`]). `{}` when rendering as plain Chart.js.
    theme_layer: Value,
    theme_scale: Value,
}

impl Chart {
    pub fn new(config: ChartConfig) -> Chart {
        Self::with_theme(config, None)
    }

    /// Build a chart whose *defaults* come from a terminal theme. Explicit colors in the config
    /// still win. `None` gives exact Chart.js defaults.
    pub fn with_theme(config: ChartConfig, theme: Option<crate::theme::Theme>) -> Chart {
        let datasets = apply_colors_plugin(&config, theme.as_ref());
        let theme_layer = theme.as_ref().map_or_else(|| json!({}), crate::theme::Theme::defaults_layer);
        let theme_scale = theme.as_ref().map_or_else(|| json!({}), crate::theme::Theme::scale_layer);
        let mut font_fallback =
            config.options.get("font").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        merge_if(&mut font_fallback, &[&DEFAULTS["font"]]);

        let mut chart = Chart {
            config,
            datasets,
            scales: vec![],
            metas: vec![],
            stacks: HashMap::new(),
            title: placeholder_block(),
            subtitle: placeholder_block(),
            legend: Legend::new(
                false,
                Position::Top,
                "center".into(),
                0.0,
                true,
                None,
                None,
                LegendLabels {
                    box_width: 40.0,
                    box_height: None,
                    padding: 10.0,
                    font: FontSpec::resolve(&Scopes::default(), &Scopes::new([&DEFAULTS["font"]])),
                    color: None,
                    use_point_style: false,
                    point_style_width: None,
                    text_align: TextAlign::Left,
                },
                LegendTitle {
                    display: false,
                    text: Label::Text(String::new()),
                    font: FontSpec::resolve(&Scopes::default(), &Scopes::new([&DEFAULTS["font"]])),
                    color: None,
                    padding: Padding::default(),
                    position: "center".into(),
                },
                vec![],
            ),
            chart_area: ChartArea::default(),
            width: 0.0,
            height: 0.0,
            dpr: 1.0,
            font_fallback,
            theme_layer,
            theme_scale,
        };
        chart.build_scales();
        chart.build_metas();
        chart.parse_all();
        chart.update_stacks();
        chart.determine_data_limits();
        chart.build_plugins();
        chart
    }

    // ------------------------------------------------------------------ scopes

    fn chart_type(&self) -> ChartType {
        self.config.chart_type
    }

    /// `chartOptionScopes()`: options, overrides[type], defaults.datasets[type], defaults.
    fn chart_scopes(&self) -> Scopes<'_> {
        let t = self.chart_type().as_str();
        Scopes::new([
            &self.config.options,
            overrides_for(t),
            &DEFAULTS["datasets"][t],
            &self.theme_layer,
            &*DEFAULTS,
        ])
    }

    fn plugin_scopes(&self, id: &str) -> Scopes<'_> {
        let t = self.chart_type().as_str();
        Scopes::new([
            &self.config.options["plugins"][id],
            &overrides_for(t)["plugins"][id],
            &self.theme_layer["plugins"][id],
            &DEFAULTS["plugins"][id],
        ])
    }

    /// `datasetScopeKeys`: dataset → options.datasets[T] → options → overrides → defaults.
    fn dataset_scopes(&self, ds: usize, kind: ChartType) -> Scopes<'_> {
        let t = kind.as_str();
        let o = &self.config.options;
        let ov = overrides_for(t);
        Scopes::new([
            &self.datasets[ds],
            &o["datasets"][t],
            o,
            &ov["datasets"][t],
            ov,
            &DEFAULTS["datasets"][t],
            &self.theme_layer,
            &*DEFAULTS,
        ])
    }

    /// `datasetElementScopeKeys(T, el)` with element prefixes.
    fn element_scopes(&self, ds: usize, kind: ChartType, el: &str) -> Scopes<'_> {
        let t = kind.as_str();
        let o = &self.config.options;
        let ov = overrides_for(t);
        let d = &*DEFAULTS;
        Scopes::new([
            &self.datasets[ds],
            &o["datasets"][t]["elements"][el],
            &o["datasets"][t],
            &o["elements"][el],
            o,
            &ov["datasets"][t]["elements"][el],
            &ov["datasets"][t],
            &ov["elements"][el],
            ov,
            &d["datasets"][t]["elements"][el],
            &d["datasets"][t],
            &d["elements"][el],
            &self.theme_layer,
            d,
        ])
        .with_prefixes(&[el, ""])
    }

    fn index_axis_for(&self, kind: ChartType, ds: Option<usize>) -> Axis {
        let t = kind.as_str();
        fn from(v: &Value) -> Option<&str> {
            v.get("indexAxis").and_then(Value::as_str)
        }
        let axis = ds
            .and_then(|i| from(&self.datasets[i]))
            .or_else(|| from(&self.config.options["datasets"][t]))
            .or_else(|| from(&self.config.options))
            .or_else(|| from(&DEFAULTS["datasets"][t]))
            .unwrap_or("x");
        if axis == "y" { Axis::Y } else { Axis::X }
    }

    // ------------------------------------------------------------------ scales

    /// `mergeScaleConfig` + scale construction.
    fn build_scales(&mut self) {
        let chart_type = self.chart_type();
        let chart_index_axis = self.index_axis_for(chart_type, None);
        let options = &self.config.options;
        let config_scales = options.get("scales").and_then(Value::as_object).cloned().unwrap_or_default();
        let chart_overrides = &overrides_for(chart_type.as_str())["scales"];
        let mut scales: Vec<(String, Value)> = Vec::new();
        let default_id =
            |axis: &str, index_axis: Axis| if axis == index_axis.as_str() { "_index_" } else { "_value_" };

        for (id, conf) in &config_scales {
            if !conf.is_object() {
                continue;
            }
            let axis = determine_axis(id, conf, &self.datasets).unwrap_or("x");
            let mut merged = json!({ "axis": axis });
            merge_if(
                &mut merged,
                &[conf, &chart_overrides[axis], &chart_overrides[default_id(axis, chart_index_axis)]],
            );
            scales.push((id.clone(), merged));
        }
        for (i, ds) in self.datasets.iter().enumerate() {
            let kind =
                ds.get("type").and_then(Value::as_str).and_then(ChartType::parse).unwrap_or(chart_type);
            let index_axis = self.index_axis_for(kind, Some(i));
            let ds_overrides = &overrides_for(kind.as_str())["scales"];
            if let Some(map) = ds_overrides.as_object() {
                for (default_scale_id, defs) in map {
                    let axis = match default_scale_id.as_str() {
                        "_index_" => index_axis.as_str(),
                        "_value_" => {
                            if index_axis == Axis::X {
                                "y"
                            } else {
                                "x"
                            }
                        }
                        other => other,
                    };
                    let id =
                        ds.get(format!("{axis}AxisID")).and_then(Value::as_str).unwrap_or(axis).to_owned();
                    let pos = match scales.iter().position(|(sid, _)| *sid == id) {
                        Some(p) => p,
                        None => {
                            scales.push((id.clone(), json!({})));
                            scales.len() - 1
                        }
                    };
                    let target = &mut scales[pos].1;
                    let empty = json!({});
                    merge_if(
                        target,
                        &[&json!({ "axis": axis }), config_scales.get(&id).unwrap_or(&empty), defs],
                    );
                }
            }
        }

        let labels = self.config.labels();
        let chart_color = self.chart_scopes().get("color").cloned().unwrap_or(json!("#666"));
        let chart_border =
            self.chart_scopes().get("borderColor").cloned().unwrap_or(json!("rgba(0,0,0,0.1)"));
        for (id, mut merged) in scales {
            let ty = merged.get("type").and_then(Value::as_str).unwrap_or("linear").to_owned();
            let type_defaults = DEFAULTS["scales"].get(&ty).cloned().unwrap_or(json!({}));
            merge_if(&mut merged, &[&type_defaults, &self.theme_scale, &DEFAULTS["scale"]]);
            let kind = if ty == "category" { ScaleKind::Category } else { ScaleKind::Linear };
            let axis = if merged["axis"] == "y" { Axis::Y } else { Axis::X };
            // Routes: grid.color → borderColor, ticks/title color → color, border.color → borderColor.
            let grid_color = merged["grid"].get("color").cloned().unwrap_or(chart_border.clone());
            let routes = json!({
                "color": chart_color,
                "ticks": { "color": chart_color },
                "title": { "color": chart_color },
                "grid": { "color": chart_border },
                "border": { "color": chart_border },
            });
            let _ = grid_color;
            let labels = match axis {
                Axis::X => self.config.data.get("xLabels"),
                Axis::Y => self.config.data.get("yLabels"),
            }
            .and_then(Value::as_array)
            .map(|a| a.iter().map(Label::from_value).collect())
            .unwrap_or_else(|| labels.clone());
            self.scales.push(Scale::new(&id, kind, axis, merged, routes, self.font_fallback.clone(), labels));
        }
    }

    fn scale_index(&self, id: &str) -> Option<usize> {
        self.scales.iter().position(|s| s.id == id)
    }

    fn first_scale_for_axis(&self, axis: Axis) -> Option<usize> {
        self.scales.iter().position(|s| s.axis == axis)
    }

    // ------------------------------------------------------------------ datasets

    fn build_metas(&mut self) {
        let chart_type = self.chart_type();
        for i in 0..self.datasets.len() {
            let ds = &self.datasets[i];
            let kind =
                ds.get("type").and_then(Value::as_str).and_then(ChartType::parse).unwrap_or(chart_type);
            let index_axis = self.index_axis_for(kind, Some(i));
            let x = ds
                .get("xAxisID")
                .and_then(Value::as_str)
                .and_then(|id| self.scale_index(id))
                .or(self.first_scale_for_axis(Axis::X));
            let y = ds
                .get("yAxisID")
                .and_then(Value::as_str)
                .and_then(|id| self.scale_index(id))
                .or(self.first_scale_for_axis(Axis::Y));
            let (Some(x_scale), Some(y_scale)) = (x, y) else { continue };
            let stack = ds.get("stack").map(crate::config::js_to_string);
            let meta = DatasetMeta {
                index: i,
                kind,
                order: ds.get("order").and_then(as_num).unwrap_or(0.0),
                visible: ds.get("hidden").and_then(Value::as_bool) != Some(true),
                label: ds.get("label").map(Label::from_value).unwrap_or(Label::Text(String::new())),
                index_axis,
                x_scale,
                y_scale,
                stack,
                stacked: None,
                parsed: vec![],
                points: vec![],
                line: None,
                bars: vec![],
                clip: None,
            };
            self.metas.push(meta);
        }
        for m in 0..self.metas.len() {
            let v = self.metas[m].v_scale();
            let st = self.scales[v].stacked();
            // isStacked: scale.stacked, or unset with a dataset `stack`.
            self.metas[m].stacked = match st {
                Some(Stacked::No) => None,
                Some(s) => Some(s),
                None if self.metas[m].stack.is_some() => Some(Stacked::Yes),
                None => None,
            };
            let ds = &self.datasets[self.metas[m].index];
            if let Some(v) = ds.get("visible") {
                let _ = v;
            }
        }
        for s in 0..self.scales.len() {
            let n = self.metas.iter().filter(|m| m.visible && (m.x_scale == s || m.y_scale == s)).count();
            self.scales[s].visible_datasets = n;
        }
    }

    fn parse_all(&mut self) {
        for m in 0..self.metas.len() {
            let parsed = self.parse_meta(m);
            self.metas[m].parsed = parsed;
        }
    }

    fn parse_meta(&mut self, m: usize) -> Vec<Parsed> {
        let meta = self.metas[m].clone();
        let ds = self.datasets[meta.index].clone();
        let parsing = ds.get("parsing").or_else(|| self.config.options.get("parsing")).cloned();
        let x_key = parsing
            .as_ref()
            .and_then(|p| p.get("xAxisKey"))
            .and_then(Value::as_str)
            .unwrap_or("x")
            .to_owned();
        let y_key = parsing
            .as_ref()
            .and_then(|p| p.get("yAxisKey"))
            .and_then(Value::as_str)
            .unwrap_or("y")
            .to_owned();

        // Object data `{label: value}` becomes `[{i: label, v: value}]`.
        let data: Vec<Value> = match &ds["data"] {
            Value::Array(a) => a.clone(),
            Value::Object(o) => {
                let (ik, vk) = if meta.index_axis == Axis::X { ("x", "y") } else { ("y", "x") };
                o.iter().map(|(k, v)| json!({ ik: k, vk: v })).collect()
            }
            _ => vec![],
        };
        let (iax, vax) = (meta.index_axis, if meta.index_axis == Axis::X { Axis::Y } else { Axis::X });
        let (is, vs) = (meta.i_scale(), meta.v_scale());
        let labels = self.scales[is].labels.clone();
        let first = data.first();
        let is_bar = meta.kind == ChartType::Bar;

        let mut out = Vec::with_capacity(data.len());
        for (i, raw) in data.iter().enumerate() {
            let mut p = Parsed::default();
            let set = |p: &mut Parsed, axis: Axis, v: Option<f64>| match axis {
                Axis::X => p.x = v,
                Axis::Y => p.y = v,
            };
            match first {
                Some(Value::Array(_)) if !is_bar => {
                    let a = raw.as_array();
                    let xr = a.and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
                    let yr = a.and_then(|a| a.get(1)).cloned().unwrap_or(Value::Null);
                    p.x = self.parse_on(meta.x_scale, &xr, i);
                    p.y = self.parse_on(meta.y_scale, &yr, i);
                }
                Some(Value::Object(_)) => {
                    let xr = raw.get(&x_key).cloned().unwrap_or(Value::Null);
                    let yr = raw.get(&y_key).cloned().unwrap_or(Value::Null);
                    if is_bar {
                        let (ir, vr) = if iax == Axis::X { (xr, yr) } else { (yr, xr) };
                        set(&mut p, iax, self.parse_on(is, &ir, i));
                        self.parse_bar_value(&mut p, &vr, vs, vax, i);
                    } else {
                        p.x = self.parse_on(meta.x_scale, &xr, i);
                        p.y = self.parse_on(meta.y_scale, &yr, i);
                    }
                }
                _ => {
                    let label_raw = labels.get(i).map(label_to_value).unwrap_or(Value::Null);
                    set(&mut p, iax, self.parse_on(is, &label_raw, i));
                    if is_bar {
                        self.parse_bar_value(&mut p, raw, vs, vax, i);
                    } else {
                        set(&mut p, vax, self.parse_on(vs, raw, i));
                    }
                }
            }
            out.push(p);
        }
        out
    }

    fn parse_on(&mut self, scale: usize, raw: &Value, index: usize) -> Option<f64> {
        let s = &mut self.scales[scale];
        if s.kind == ScaleKind::Category
            && !(s
                .labels
                .get(index)
                .is_some_and(|l| matches!((l, raw), (Label::Text(t), Value::String(r)) if t == r)))
        {
            s.add_label_if_missing(raw);
        }
        s.parse(raw, Some(index))
    }

    fn parse_bar_value(&mut self, p: &mut Parsed, raw: &Value, vs: usize, vax: Axis, i: usize) {
        if let Value::Array(a) = raw {
            let s = self.scales[vs].parse(a.first().unwrap_or(&Value::Null), Some(i));
            let e = self.scales[vs].parse(a.get(1).unwrap_or(&Value::Null), Some(i));
            if let (Some(start), Some(end)) = (s, e) {
                let (min, max) = (start.min(end), start.max(end));
                let (mut bs, mut be) = (min, max);
                if min.abs() > max.abs() {
                    bs = max;
                    be = min;
                }
                match vax {
                    Axis::X => p.x = Some(be),
                    Axis::Y => p.y = Some(be),
                }
                p.custom = Some(FloatBar { bar_start: bs, bar_end: be, start, end, min, max });
            }
        } else {
            let v = self.scales[vs].parse(raw, Some(i));
            match vax {
                Axis::X => p.x = v,
                Axis::Y => p.y = v,
            }
        }
    }

    /// Visible metas sorted by `order` then index (`getSortedVisibleDatasetMetas`).
    fn sorted_metas(&self, visible_only: bool) -> Vec<usize> {
        let mut v: Vec<usize> =
            (0..self.metas.len()).filter(|&m| !visible_only || self.metas[m].visible).collect();
        v.sort_by(|&a, &b| {
            let (ma, mb) = (&self.metas[a], &self.metas[b]);
            ma.order.partial_cmp(&mb.order).unwrap_or(std::cmp::Ordering::Equal).then(ma.index.cmp(&mb.index))
        });
        v
    }

    fn stack_key(&self, m: usize) -> String {
        let meta = &self.metas[m];
        let st = meta.stack.clone().unwrap_or_else(|| meta.kind.as_str().to_owned());
        format!("{}.{}.{}", self.scales[meta.i_scale()].id, self.scales[meta.v_scale()].id, st)
    }

    fn update_stacks(&mut self) {
        self.stacks.clear();
        for m in 0..self.metas.len() {
            if self.metas[m].stacked.is_none() {
                continue;
            }
            let key = self.stack_key(m);
            let meta = &self.metas[m];
            let (iax, vax) = (meta.index_axis, if meta.index_axis == Axis::X { Axis::Y } else { Axis::X });
            for p in &meta.parsed {
                let Some(iv) = p.get(iax) else { continue };
                let entry = self.stacks.entry((key.clone(), iv.to_bits())).or_default();
                entry.insert(meta.index, p.get(vax));
            }
        }
    }

    /// `applyStack`: add the values of earlier visible datasets in the same stack.
    fn apply_stack(&self, m: usize, parsed: &Parsed, single: bool, all: bool) -> Option<f64> {
        let meta = &self.metas[m];
        let vax = if meta.index_axis == Axis::X { Axis::Y } else { Axis::X };
        let mut value = parsed.get(vax)?;
        let iv = parsed.get(meta.index_axis)?;
        let key = (self.stack_key(m), iv.to_bits());
        let Some(values) = self.stacks.get(&key) else { return Some(value) };
        let mut found = false;
        for k in self.sorted_metas(true) {
            let ds = self.metas[k].index;
            if ds == meta.index {
                found = true;
                if all {
                    continue;
                }
                break;
            }
            if let Some(Some(other)) = values.get(&ds)
                && other.is_finite()
                && (single || value == 0.0 || sign(value) == sign(*other))
            {
                value += other;
            }
        }
        if !found && !all {
            return Some(0.0);
        }
        Some(value)
    }

    /// The stack's top/bottom dataset indices for an index value (`_top` / `_bottom`).
    fn stack_extremes(&self, m: usize, iv: f64) -> (Option<usize>, Option<usize>) {
        let key = (self.stack_key(m), iv.to_bits());
        let Some(values) = self.stacks.get(&key) else { return (None, None) };
        let vs = self.metas[m].v_scale();
        let kind = self.metas[m].kind;
        let mut top = None;
        let mut bottom = None;
        let matching: Vec<usize> = self
            .sorted_metas(true)
            .into_iter()
            .filter(|&k| {
                self.metas[k].kind == kind && (self.metas[k].x_scale == vs || self.metas[k].y_scale == vs)
            })
            .collect();
        for &k in matching.iter().rev() {
            let ds = self.metas[k].index;
            if let Some(Some(v)) = values.get(&ds) {
                if top.is_none() && *v > 0.0 {
                    top = Some(ds);
                }
                if bottom.is_none() && *v < 0.0 {
                    bottom = Some(ds);
                }
            }
        }
        (top, bottom)
    }

    /// `getMinMax(scale, canStack)` for one meta.
    fn meta_range(&self, m: usize, scale: usize) -> (f64, f64) {
        let meta = &self.metas[m];
        let axis = self.scales[scale].axis;
        let other = if scale == meta.i_scale() { meta.v_scale() } else { meta.i_scale() };
        let other_axis = self.scales[other].axis;
        let (omin, omax, omin_def, omax_def) = self.scales[other].user_bounds();
        let omin = if omin_def { omin } else { f64::NEG_INFINITY };
        let omax = if omax_def { omax } else { f64::INFINITY };
        let use_stack = meta.stacked.is_some() && meta.visible && scale == meta.v_scale();
        let single = meta.stacked == Some(Stacked::Single);
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        for p in &meta.parsed {
            let Some(v) = p.get(axis).filter(|v| v.is_finite()) else { continue };
            if let Some(ov) = p.get(other_axis)
                && (omin > ov || omax < ov)
            {
                continue;
            }
            let value = if use_stack { self.apply_stack(m, p, single, false).unwrap_or(f64::NAN) } else { v };
            if !value.is_nan() {
                min = min.min(value);
                max = max.max(value);
            }
            if meta.kind == ChartType::Bar
                && scale == meta.v_scale()
                && let Some(c) = p.custom
            {
                min = min.min(c.min);
                max = max.max(c.max);
            }
        }
        (min, max)
    }

    fn determine_data_limits(&mut self) {
        for s in 0..self.scales.len() {
            let ranges: Vec<(f64, f64)> = self
                .sorted_metas(true)
                .into_iter()
                .filter(|&m| self.metas[m].x_scale == s || self.metas[m].y_scale == s)
                .map(|m| self.meta_range(m, s))
                .collect();
            self.scales[s].determine_data_limits(&ranges);
        }
    }

    // ------------------------------------------------------------------ plugins

    fn build_plugins(&mut self) {
        let global_font = Scopes::new([&self.font_fallback]);
        let chart_color = self.chart_scopes().color_at("color", None);
        let block = |s: Scopes<'_>| TextBlock {
            display: s.bool_or("display", false),
            text: s.get("text").map(Label::from_value).unwrap_or(Label::Text(String::new())),
            font: FontSpec::resolve(&s.child("font"), &global_font),
            color: s.color_at("color", None).or(chart_color),
            padding: Padding::from_value(s.get("padding")),
            align: s.str("align").unwrap_or("center").to_owned(),
            position: s.str("position").and_then(Position::parse).unwrap_or(Position::Top),
            weight: s.num_or("weight", 0.0),
            full_size: s.bool_or("fullSize", true),
            geom: Default::default(),
        };
        self.title = block(self.plugin_scopes("title"));
        self.subtitle = block(self.plugin_scopes("subtitle"));

        let l = self.plugin_scopes("legend");
        let labels = l.child("labels");
        let use_point_style = labels.bool_or("usePointStyle", false);
        let legend_labels = LegendLabels {
            box_width: labels.num_or("boxWidth", 40.0),
            box_height: labels.num("boxHeight"),
            padding: labels.num_or("padding", 10.0),
            font: FontSpec::resolve(&labels.child("font"), &global_font),
            color: labels.color_at("color", None).or(chart_color),
            use_point_style,
            point_style_width: labels.num("pointStyleWidth"),
            text_align: match labels.str("textAlign") {
                Some("center") => TextAlign::Center,
                Some("right") => TextAlign::Right,
                _ => TextAlign::Left,
            },
        };
        let t = l.child("title");
        let legend_title = LegendTitle {
            display: t.bool_or("display", false),
            text: t.get("text").map(Label::from_value).unwrap_or(Label::Text(String::new())),
            font: FontSpec::resolve(&t.child("font"), &global_font),
            color: t.color_at("color", None).or(chart_color),
            padding: Padding::from_value(t.get("padding")),
            position: t.str("position").unwrap_or("center").to_owned(),
        };
        let point_style_override = labels.get("pointStyle").map(|v| PointStyle::from_value(Some(v)));
        let use_border_radius = labels.bool_or("useBorderRadius", false);
        let label_border_radius = labels.get("borderRadius").cloned();
        let mut items = Vec::new();
        for m in self.sorted_metas(false) {
            let meta = &self.metas[m];
            let ds = meta.index;
            // getStyle(): line datasets use the dataset (line) element unless usePointStyle.
            let (el, idx) = match meta.kind {
                ChartType::Line if !use_point_style => ("line", None),
                ChartType::Line => ("point", Some(0)),
                ChartType::Bar => ("bar", Some(0)),
            };
            let s = self.element_scopes(ds, meta.kind, el);
            let bw = Padding::from_value(s.get_at("borderWidth", idx));
            items.push(LegendItem {
                text: meta.label.clone(),
                fill_style: s.color_at("backgroundColor", idx),
                stroke_style: s.color_at("borderColor", idx),
                line_width: (bw.width() + bw.height()) / 4.0,
                line_cap: parse_cap(s.str("borderCapStyle")),
                line_join: parse_join(s.str("borderJoinStyle")),
                line_dash: s.num_list("borderDash"),
                line_dash_offset: s.num_or("borderDashOffset", 0.0),
                point_style: point_style_override
                    .unwrap_or_else(|| PointStyle::from_value(s.get_at("pointStyle", idx))),
                rotation: s.num_at("rotation", idx).unwrap_or(0.0),
                hidden: !meta.visible,
                border_radius: use_border_radius.then(|| {
                    Corners::from_value(label_border_radius.as_ref().or(s.get_at("borderRadius", idx)))
                }),
                dataset_index: ds,
            });
        }
        if l.bool_or("reverse", false) {
            items.reverse();
        }
        self.legend = Legend::new(
            l.bool_or("display", true),
            l.str("position").and_then(Position::parse).unwrap_or(Position::Top),
            l.str("align").unwrap_or("center").to_owned(),
            l.num_or("weight", 1000.0),
            l.bool_or("fullSize", true),
            l.num("maxWidth"),
            l.num("maxHeight"),
            legend_labels,
            legend_title,
            items,
        );
    }

    // ------------------------------------------------------------------ element options

    fn point_options(&self, m: usize, i: usize) -> PointOptions {
        let meta = &self.metas[m];
        let s = self.element_scopes(meta.index, meta.kind, "point");
        let idx = Some(i);
        PointOptions {
            radius: s.num_at("radius", idx).unwrap_or(3.0),
            hover_radius: s.num_at("hoverRadius", idx).unwrap_or(4.0),
            hit_radius: s.num_at("hitRadius", idx).unwrap_or(1.0),
            point_style: PointStyle::from_value(s.get_at("pointStyle", idx)),
            rotation: s.num_at("rotation", idx).unwrap_or(0.0),
            border_width: s.num_at("borderWidth", idx).unwrap_or(1.0),
            background_color: s.color_at("backgroundColor", idx),
            border_color: s.color_at("borderColor", idx),
        }
    }

    fn line_options(&self, m: usize) -> LineOptions {
        let meta = &self.metas[m];
        let s = self.element_scopes(meta.index, meta.kind, "line");
        let ds = self.dataset_scopes(meta.index, meta.kind);
        let span_gaps = match ds.get("spanGaps").or_else(|| s.get("spanGaps")) {
            Some(Value::Bool(true)) => SpanGaps::Yes,
            Some(Value::Number(n)) => SpanGaps::Max(n.as_f64().unwrap_or(0.0)),
            _ => SpanGaps::No,
        };
        let show_line = ds.bool_or("showLine", true);
        LineOptions {
            border_width: if show_line { s.num_or("borderWidth", 3.0) } else { 0.0 },
            border_color: s.color_at("borderColor", None),
            background_color: s.color_at("backgroundColor", None),
            cap: parse_cap(s.str("borderCapStyle")),
            join: parse_join(s.str("borderJoinStyle")),
            dash: s.num_list("borderDash"),
            dash_offset: s.num_or("borderDashOffset", 0.0),
            tension: s.num_or("tension", 0.0),
            monotone: s.str("cubicInterpolationMode") == Some("monotone"),
            stepped: match s.get("stepped") {
                Some(Value::Bool(true)) => Stepped::Before,
                Some(Value::String(v)) if v == "before" => Stepped::Before,
                Some(Value::String(v)) if v == "after" => Stepped::After,
                Some(Value::String(v)) if v == "middle" => Stepped::Middle,
                _ => Stepped::No,
            },
            span_gaps,
            cap_bezier_points: s.bool_or("capBezierPoints", true),
        }
    }

    fn bar_options(&self, m: usize, i: usize) -> BarOptions {
        let meta = &self.metas[m];
        let s = self.element_scopes(meta.index, meta.kind, "bar");
        let idx = Some(i);
        let radius = s.get_at("borderRadius", idx);
        BarOptions {
            background_color: s.color_at("backgroundColor", idx),
            border_color: s.color_at("borderColor", idx),
            border_width: Padding::from_value(s.get_at("borderWidth", idx)),
            border_radius: Corners::from_value(radius),
            border_radius_is_object: radius.is_some_and(Value::is_object),
            border_skipped: match s.get_at("borderSkipped", idx) {
                Some(Value::Bool(false)) | Some(Value::Null) | None => None,
                Some(Value::Bool(true)) => Some("true".into()),
                Some(Value::String(v)) => Some(v.clone()),
                Some(_) => Some("start".into()),
            },
            inflate_amount: s.get_at("inflateAmount", idx).and_then(as_num),
        }
    }

    // ------------------------------------------------------------------ update (after layout)

    fn max_overflow(&self) -> f64 {
        let mut pad: f64 = 0.0;
        for m in 0..self.metas.len() {
            let meta = &self.metas[m];
            if meta.kind != ChartType::Line {
                continue;
            }
            let n = meta.parsed.len();
            if n == 0 {
                continue;
            }
            // On the first update the line element has no options yet, so only points count.
            let first = self.point_options(m, 0).size();
            let last = self.point_options(m, n - 1).size();
            pad = pad.max(first.max(last) / 2.0);
        }
        pad
    }

    fn update_datasets(&mut self) {
        for m in 0..self.metas.len() {
            match self.metas[m].kind {
                ChartType::Line => self.update_line(m),
                ChartType::Bar => self.update_bar(m),
            }
            self.metas[m].clip = self.dataset_clip(m);
        }
    }

    fn dataset_clip(&self, m: usize) -> Option<Padding> {
        let meta = &self.metas[m];
        let ds = self.dataset_scopes(meta.index, meta.kind);
        if let Some(v) = ds.get("clip") {
            if v == &Value::Bool(false) {
                return None;
            }
            return Some(Padding::from_value(Some(v)));
        }
        let overflow = match meta.kind {
            ChartType::Bar => 0.0,
            ChartType::Line => {
                let border = self.metas[m].line.as_ref().map_or(0.0, |l| l.options.border_width);
                let n = meta.parsed.len();
                if n == 0 {
                    border
                } else {
                    border.max(self.point_options(m, 0).size()).max(self.point_options(m, n - 1).size()) / 2.0
                }
            }
        };
        let clip_for = |s: &Scale| {
            let (_, _, min_def, max_def) = s.user_bounds();
            let (min, max) = (if min_def { 0.0 } else { overflow }, if max_def { 0.0 } else { overflow });
            if s.reverse() { (max, min) } else { (min, max) }
        };
        let (xs, xe) = clip_for(&self.scales[meta.x_scale]);
        let (ys, ye) = clip_for(&self.scales[meta.y_scale]);
        Some(Padding { top: ye, right: xe, bottom: ys, left: xs })
    }

    fn update_line(&mut self, m: usize) {
        let meta = self.metas[m].clone();
        let (is, vs) = (meta.i_scale(), meta.v_scale());
        let (iax, vax) = (meta.index_axis, if meta.index_axis == Axis::X { Axis::Y } else { Axis::X });
        let line_opts = self.line_options(m);
        let max_gap = match line_opts.span_gaps {
            SpanGaps::Max(n) => n,
            _ => f64::INFINITY,
        };
        let single = meta.stacked == Some(Stacked::Single);
        let mut points = Vec::with_capacity(meta.parsed.len());
        let mut prev: Option<Parsed> = None;
        for (i, p) in meta.parsed.iter().enumerate() {
            let ival = p.get(iax).unwrap_or(f64::NAN);
            let i_pixel = self.scales[is].pixel_for_value(ival);
            let null_data = p.get(vax).is_none();
            let v_pixel = if null_data {
                self.scales[vs].base_pixel()
            } else {
                let v =
                    if meta.stacked.is_some() { self.apply_stack(m, p, single, false) } else { p.get(vax) };
                self.scales[vs].pixel_for_value(v.unwrap_or(f64::NAN))
            };
            let (x, y) = if iax == Axis::X { (i_pixel, v_pixel) } else { (v_pixel, i_pixel) };
            let stop =
                i > 0 && prev.is_some_and(|pp| (ival - pp.get(iax).unwrap_or(f64::NAN)).abs() > max_gap);
            points.push(Point {
                x,
                y,
                skip: i_pixel.is_nan() || v_pixel.is_nan() || null_data,
                stop,
                cp1x: x,
                cp1y: y,
                cp2x: x,
                cp2y: y,
                options: self.point_options(m, i),
            });
            prev = Some(*p);
        }
        self.metas[m].points = points;
        self.metas[m].line = Some(Line { options: line_opts });
    }

    fn update_bar(&mut self, m: usize) {
        let meta = self.metas[m].clone();
        let (is, vs) = (meta.i_scale(), meta.v_scale());
        let iax = meta.index_axis;
        let vax = if iax == Axis::X { Axis::Y } else { Axis::X };
        let horizontal = self.scales[vs].is_horizontal();
        let ds = self.dataset_scopes(meta.index, meta.kind);
        let bar_thickness = ds.get("barThickness").cloned();
        let category_pct = ds.num_or("categoryPercentage", 0.8);
        let bar_pct = ds.num_or("barPercentage", 0.9);
        let grouped = ds.bool_or("grouped", true);
        let max_bar_thickness = ds.num("maxBarThickness").unwrap_or(f64::INFINITY);
        let min_bar_length = ds.num_or("minBarLength", 0.0);
        let base_value = ds.num("base");
        let skip_null = ds.bool_or("skipNull", false);

        // _getRuler
        let pixels: Vec<f64> = meta
            .parsed
            .iter()
            .map(|p| self.scales[is].pixel_for_value(p.get(iax).unwrap_or(f64::NAN)))
            .collect();
        let thickness_num = bar_thickness.as_ref().and_then(as_num);
        let flex = bar_thickness.as_ref().and_then(Value::as_str) == Some("flex");
        let ruler_min = thickness_num.unwrap_or_else(|| self.min_sample_size(m));
        let ratio = if thickness_num.is_some() { 1.0 } else { category_pct * bar_pct };
        let stack_count = self.stacks_for(m, None, None).len() as f64;
        let i_scale = &self.scales[is];
        let (ruler_start, ruler_end) = (i_scale.start_pixel(), i_scale.end_pixel());

        let base_pixel = self.scales[vs].base_pixel();
        let mut bars = Vec::with_capacity(meta.parsed.len());
        for (i, p) in meta.parsed.iter().enumerate() {
            let options = self.bar_options(m, i);
            // value pixels
            let (base, head, size) = if p.get(vax).is_none() {
                (base_pixel, base_pixel, 0.0)
            } else {
                self.bar_value_pixels(m, p, base_value, min_bar_length)
            };
            // index pixels
            let (center, isize) = if grouped {
                let count =
                    if skip_null { self.stacks_for(m, None, Some(i)).len() as f64 } else { stack_count };
                let (chunk, r, start) = if flex {
                    let curr = pixels[i];
                    let mut prev = if i > 0 { Some(pixels[i - 1]) } else { None };
                    let mut next = if i + 1 < pixels.len() { Some(pixels[i + 1]) } else { None };
                    if prev.is_none() {
                        prev = Some(curr - next.map_or(ruler_end - ruler_start, |n| n - curr));
                    }
                    let pv = prev.unwrap_or(curr);
                    if next.is_none() {
                        next = Some(curr + curr - pv);
                    }
                    let nx = next.unwrap_or(curr);
                    let start = curr - (curr - pv.min(nx)) / 2.0 * category_pct;
                    let size = (nx - pv).abs() / 2.0 * category_pct;
                    (size / count, bar_pct, start)
                } else {
                    let (size, r) = match thickness_num {
                        Some(t) => (t * count, 1.0),
                        None => (ruler_min * category_pct, bar_pct),
                    };
                    (size / count, r, pixels[i] - size / 2.0)
                };
                let stack_index = self.stack_index(m, if skip_null { Some(i) } else { None }) as f64;
                (start + chunk * stack_index + chunk / 2.0, max_bar_thickness.min(chunk * r))
            } else {
                (pixels[i], max_bar_thickness.min(ruler_min * ratio))
            };

            let floating = p.custom.is_some();
            let (top, bottom) = match p.get(iax) {
                Some(iv) if meta.stacked.is_some() => self.stack_extremes(m, iv),
                _ => (None, None),
            };
            let has_stack = meta.stacked.is_some();
            let enable_border_radius =
                !has_stack || floating || Some(meta.index) == top || Some(meta.index) == bottom;
            let mut bar = Bar {
                x: if horizontal { head } else { center },
                y: if horizontal { center } else { head },
                base,
                width: if horizontal { size.abs() } else { isize },
                height: if horizontal { isize } else { size.abs() },
                horizontal,
                border_skipped: Edges::default(),
                enable_border_radius,
                inflate_amount: match options.inflate_amount {
                    Some(v) => v,
                    None => {
                        if ratio == 1.0 {
                            0.33
                        } else {
                            0.0
                        }
                    }
                },
                options,
            };
            set_border_skipped(&mut bar, has_stack, top, bottom, meta.index);
            bars.push(bar);
        }
        self.metas[m].bars = bars;
    }

    /// `_calculateBarValuePixels` → (base, head, size).
    fn bar_value_pixels(
        &self,
        m: usize,
        p: &Parsed,
        base_value: Option<f64>,
        min_bar_length: f64,
    ) -> (f64, f64, f64) {
        let meta = &self.metas[m];
        let vs = &self.scales[meta.v_scale()];
        let vax = if meta.index_axis == Axis::X { Axis::Y } else { Axis::X };
        let actual_base = base_value.unwrap_or(0.0);
        let floating = p.custom;
        let mut value = p.get(vax).unwrap_or(0.0);
        let mut start = 0.0;
        let mut length = if meta.stacked.is_some() {
            self.apply_stack(m, p, meta.stacked == Some(Stacked::Single), false).unwrap_or(value)
        } else {
            value
        };
        if length != value {
            start = length - value;
            length = value;
        }
        if let Some(c) = floating {
            value = c.bar_start;
            length = c.bar_end - c.bar_start;
            if value != 0.0 && sign(value) != sign(c.bar_end) {
                start = 0.0;
            }
            start += value;
        }
        let start_value = match (base_value, floating) {
            (Some(b), None) => b,
            _ => start,
        };
        let mut base = vs.pixel_for_value(start_value);
        let mut head = vs.pixel_for_value(start + length);
        let mut size = head - base;
        if size.abs() < min_bar_length {
            let bsign = if size != 0.0 {
                sign(size)
            } else {
                (if vs.is_horizontal() { 1.0 } else { -1.0 })
                    * (if vs.min >= actual_base { 1.0 } else { -1.0 })
            };
            size = bsign * min_bar_length;
            if value == actual_base {
                base -= size / 2.0;
            }
            let (sp, ep) = (vs.pixel_for_decimal(0.0), vs.pixel_for_decimal(1.0));
            base = base.max(sp.min(ep)).min(sp.max(ep));
            head = base + size;
        }
        if base == vs.pixel_for_value(actual_base) {
            let half_grid = sign(size) * vs.line_width_for_value(actual_base) / 2.0;
            base += half_grid;
            size -= half_grid;
        }
        (base, head, size)
    }

    /// `computeMinSampleSize`: the smallest pixel gap between bar positions or ticks.
    fn min_sample_size(&self, m: usize) -> f64 {
        let meta = &self.metas[m];
        let is = meta.i_scale();
        let scale = &self.scales[is];
        let iax = scale.axis;
        let mut values: Vec<f64> = self
            .sorted_metas(true)
            .into_iter()
            .filter(|&k| self.metas[k].kind == meta.kind && self.metas[k].i_scale() == is)
            .flat_map(|k| {
                self.metas[k].parsed.iter().filter_map(move |p| p.get(iax)).filter(|v| v.is_finite())
            })
            .collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        values.dedup();
        let mut min = scale.length;
        let mut prev: Option<f64> = None;
        let visit = |curr: f64, prev: &mut Option<f64>, min: &mut f64| {
            if curr == 32767.0 || curr == -32768.0 {
                return;
            }
            if let Some(p) = *prev {
                let d = (curr - p).abs();
                *min = min.min(if d != 0.0 { d } else { *min });
            }
            *prev = Some(curr);
        };
        for v in values {
            visit(scale.pixel_for_value(v), &mut prev, &mut min);
        }
        prev = None;
        for i in 0..scale.ticks.len() {
            if let Some(px) = scale.pixel_for_tick(i) {
                visit(px, &mut prev, &mut min);
            }
        }
        min
    }

    /// `_getStacks(last, dataIndex)`: the stack groups sharing the index scale.
    fn stacks_for(&self, m: usize, last: Option<usize>, data_index: Option<usize>) -> Vec<Option<String>> {
        let meta = &self.metas[m];
        let is = meta.i_scale();
        let stacked = self.scales[is].stacked();
        let iax = meta.index_axis;
        let iv = data_index.and_then(|i| meta.parsed.get(i)).and_then(|p| p.get(iax));
        let mut stacks: Vec<Option<String>> = Vec::new();
        for k in self.sorted_metas(true) {
            let other = &self.metas[k];
            if other.kind != meta.kind || other.i_scale() != is {
                continue;
            }
            if !self.dataset_scopes(other.index, other.kind).bool_or("grouped", true) {
                continue;
            }
            if data_index.is_some() {
                let vax = if other.index_axis == Axis::X { Axis::Y } else { Axis::X };
                let val =
                    other.parsed.iter().find(|p| p.get(other.index_axis) == iv).and_then(|p| p.get(vax));
                if val.is_none_or(f64::is_nan) {
                    continue;
                }
            }
            if stacked == Some(Stacked::No)
                || !stacks.contains(&other.stack)
                || (stacked.is_none() && other.stack.is_none())
            {
                stacks.push(other.stack.clone());
            }
            if Some(other.index) == last {
                break;
            }
        }
        if stacks.is_empty() {
            stacks.push(None);
        }
        stacks
    }

    fn stack_index(&self, m: usize, data_index: Option<usize>) -> usize {
        let meta = &self.metas[m];
        let stacks = self.stacks_for(m, Some(meta.index), data_index);
        match &meta.stack {
            Some(_) => stacks.iter().position(|s| *s == meta.stack).unwrap_or(stacks.len() - 1),
            None => stacks.len() - 1,
        }
    }

    // ------------------------------------------------------------------ render

    /// Lay out and draw onto `c` (the whole canvas: `c.width() × c.height()` logical px).
    pub fn draw(&mut self, c: &mut dyn Canvas) {
        self.width = f64::from(c.width());
        self.height = f64::from(c.height());
        for s in &mut self.scales {
            s.chart_width = self.width;
            s.chart_height = self.height;
            s.dpr = self.dpr;
        }
        let auto_padding = self.chart_scopes().child("layout").bool_or("autoPadding", true);
        let min_padding = if auto_padding { self.max_overflow() } else { 0.0 };
        let padding = Padding::from_value(self.chart_scopes().child("layout").get("padding"));

        // Layout. Order of boxes = plugin start order (legend, subtitle, title), then scales.
        let (w, h) = (self.width, self.height);
        let area = {
            let mut items: Vec<&mut dyn LayoutItem> = Vec::new();
            items.push(&mut self.legend);
            items.push(&mut self.subtitle);
            items.push(&mut self.title);
            for s in &mut self.scales {
                items.push(s);
            }
            layout::update(&mut items, w, h, padding, min_padding, c)
        };
        self.chart_area = area;
        for s in &mut self.scales {
            s.configure();
        }
        self.update_datasets();

        // Draw: layers with z ≤ 0 (boxes), then datasets, in Chart.js order.
        self.legend.draw(c);
        self.subtitle.draw(c);
        self.title.draw(c);
        for s in &mut self.scales {
            s.draw(c, &area);
        }
        let order = self.sorted_metas(true);
        for &m in order.iter().rev() {
            self.draw_dataset(c, m);
        }
    }

    fn draw_dataset(&mut self, c: &mut dyn Canvas, m: usize) {
        let area = self.chart_area;
        let meta = &mut self.metas[m];
        if let Some(clip) = meta.clip {
            clip_area(
                c,
                area.left - clip.left,
                area.top - clip.top,
                area.right + clip.right,
                area.bottom + clip.bottom,
            );
        }
        match meta.kind {
            ChartType::Line => {
                if let Some(line) = meta.line.clone() {
                    line.update_control_points(&mut meta.points, &area, meta.index_axis == Axis::X);
                    line.draw(c, &meta.points);
                }
                for p in &meta.points {
                    p.draw(c, &area);
                }
            }
            ChartType::Bar => {
                let vax = if meta.index_axis == Axis::X { Axis::Y } else { Axis::X };
                for (b, p) in meta.bars.iter().zip(&meta.parsed) {
                    if p.get(vax).is_some() {
                        b.draw(c);
                    }
                }
            }
        }
        if meta.clip.is_some() {
            c.restore();
        }
    }

    /// Fill the canvas with a color first (Chart.js leaves the canvas transparent).
    pub fn fill_background(c: &mut dyn Canvas, color: Color) {
        c.save();
        c.set_fill_style(Paint::Solid(color));
        c.fill_rect(0.0, 0.0, c.width(), c.height());
        c.restore();
    }

    /// `options.aspectRatio` (default 2), used to derive a height from a width.
    pub fn aspect_ratio(&self) -> f64 {
        self.chart_scopes().num("aspectRatio").filter(|r| *r > 0.0).unwrap_or(2.0)
    }
}

fn placeholder_block() -> TextBlock {
    TextBlock {
        display: false,
        text: Label::Text(String::new()),
        font: FontSpec::resolve(&Scopes::default(), &Scopes::new([&DEFAULTS["font"]])),
        color: None,
        padding: Padding::default(),
        align: "center".into(),
        position: Position::Top,
        weight: 0.0,
        full_size: true,
        geom: Default::default(),
    }
}

fn label_to_value(l: &Label) -> Value {
    match l {
        Label::Text(s) => Value::String(s.clone()),
        Label::Lines(v) => Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
    }
}

fn parse_cap(s: Option<&str>) -> LineCap {
    match s {
        Some("round") => LineCap::Round,
        Some("square") => LineCap::Square,
        _ => LineCap::Butt,
    }
}

fn parse_join(s: Option<&str>) -> LineJoin {
    match s {
        Some("round") => LineJoin::Round,
        Some("bevel") => LineJoin::Bevel,
        _ => LineJoin::Miter,
    }
}

/// `determineAxis(id, scaleOptions...)`.
fn determine_axis<'a>(id: &'a str, conf: &'a Value, datasets: &[Value]) -> Option<&'a str> {
    if matches!(id, "x" | "y" | "r") {
        return Some(id);
    }
    if let Some(a) = conf.get("axis").and_then(Value::as_str) {
        return Some(if a == "x" { "x" } else { "y" });
    }
    match conf.get("position").and_then(Value::as_str) {
        Some("top" | "bottom") => return Some("x"),
        Some("left" | "right") => return Some("y"),
        _ => {}
    }
    for ds in datasets {
        if ds.get("xAxisID").and_then(Value::as_str) == Some(id) {
            return Some("x");
        }
        if ds.get("yAxisID").and_then(Value::as_str) == Some(id) {
            return Some("y");
        }
    }
    match id.chars().next().map(|c| c.to_ascii_lowercase()) {
        Some('x') if id.len() > 1 => Some("x"),
        Some('y') if id.len() > 1 => Some("y"),
        _ => None,
    }
}

/// The `colors` plugin: assign the default palette when nothing defines colors. With a theme,
/// the palette is the theme's (terminal-derived) series colors.
fn apply_colors_plugin(config: &ChartConfig, theme: Option<&crate::theme::Theme>) -> Vec<Value> {
    let mut datasets = config.datasets().to_vec();
    let o = &config.options;
    let opts = &o["plugins"]["colors"];
    if opts.get("enabled").and_then(Value::as_bool) == Some(false) {
        return datasets;
    }
    let has = |v: &Value| {
        v.get("borderColor").is_some_and(|c| !c.is_null())
            || v.get("backgroundColor").is_some_and(|c| !c.is_null())
    };
    let defined = datasets.iter().any(has)
        || has(o)
        || o.get("elements").and_then(Value::as_object).is_some_and(|e| e.values().any(has));
    let force = opts.get("forceOverride").and_then(Value::as_bool) == Some(true);
    if defined && !force {
        return datasets;
    }
    for (i, ds) in datasets.iter_mut().enumerate() {
        if let Some(obj) = ds.as_object_mut() {
            let (border, fill) = match theme {
                Some(t) => (t.border_color(i), t.fill_color(i)),
                None => (
                    BORDER_COLORS[i % BORDER_COLORS.len()].to_owned(),
                    BACKGROUND_COLORS[i % BACKGROUND_COLORS.len()].to_owned(),
                ),
            };
            obj.insert("borderColor".into(), json!(border));
            obj.insert("backgroundColor".into(), json!(fill));
        }
    }
    datasets
}

/// `setBorderSkipped`.
fn set_border_skipped(
    bar: &mut Bar,
    has_stack: bool,
    top: Option<usize>,
    bottom: Option<usize>,
    index: usize,
) {
    let Some(edge) = bar.options.border_skipped.clone() else {
        bar.border_skipped = Edges::default();
        return;
    };
    if edge == "true" {
        bar.border_skipped = Edges { top: true, right: true, bottom: true, left: true };
        return;
    }
    // borderProps
    let (start, end, reverse) = if bar.horizontal {
        ("left", "right", bar.base > bar.x)
    } else {
        ("bottom", "top", bar.base < bar.y)
    };
    let (top_e, bottom_e) = if reverse { ("end", "start") } else { ("start", "end") };
    let mut res = Edges::default();
    let mut mark = |e: &str| match e {
        "top" => res.top = true,
        "right" => res.right = true,
        "bottom" => res.bottom = true,
        "left" => res.left = true,
        _ => {}
    };
    let parse_edge = |e: &str| -> String {
        let swap = |o: &str, a: &str, b: &str| {
            if o == a {
                b.to_owned()
            } else if o == b {
                a.to_owned()
            } else {
                o.to_owned()
            }
        };
        let start_end = |v: &str, s: &str, e: &str| match v {
            "start" => s.to_owned(),
            "end" => e.to_owned(),
            other => other.to_owned(),
        };
        if reverse {
            let swapped = swap(e, start, end);
            start_end(&swapped, end, start)
        } else {
            start_end(e, start, end)
        }
    };
    let mut edge = edge;
    if edge == "middle" && has_stack {
        bar.enable_border_radius = true;
        if top.unwrap_or(0) == index {
            edge = top_e.to_owned();
        } else if bottom.unwrap_or(0) == index {
            edge = bottom_e.to_owned();
        } else {
            mark(&parse_edge(bottom_e));
            edge = top_e.to_owned();
        }
    }
    mark(&parse_edge(&edge));
    bar.border_skipped = res;
    let _ = is_between;
}
