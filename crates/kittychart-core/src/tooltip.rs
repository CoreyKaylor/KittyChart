//! The tooltip plugin (Chart.js `plugin.tooltip.js`): title, body and footer for the active
//! elements, laid out next to them with a caret, drawn over the chart.
//!
//! Content comes from Chart.js's default callbacks, or from scriptable `callbacks.*` options
//! (see [`crate::script`]): `title`, `label`, `footer` and their `before*`/`after*` variants,
//! `labelColor` and `labelTextColor`. Callbacks receive Chart.js's arguments as `__args`.

use kittychart_canvas::{Canvas, Color, LineJoin, Paint, TextAlign, TextBaseline};
use serde_json::{Value, json};

use crate::chart::Chart;
use crate::config::{ChartType, js_to_string};
use crate::draw::{PointStyle, draw_point, rounded_rect_path};
use crate::interaction::{ActiveElement, InteractionOptions, Mode, arc_center, tooltip_position};
use crate::options::{Corners, FontSpec, Padding, Scopes, as_num};

/// One active element's tooltip data (`TooltipItem`).
#[derive(Debug, Clone)]
pub struct TooltipItem {
    pub element: ActiveElement,
    pub label: String,
    pub formatted_value: String,
}

#[derive(Debug, Clone, Default)]
struct BodyItem {
    before: Vec<String>,
    lines: Vec<String>,
    after: Vec<String>,
}

#[derive(Debug, Clone)]
struct LabelColor {
    border: Option<Paint>,
    background: Option<Paint>,
    border_width: f64,
    dash: Vec<f64>,
    dash_offset: f64,
    border_radius: Corners,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum XAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YAlign {
    Top,
    Center,
    Bottom,
}

/// The resolved tooltip options.
#[derive(Debug, Clone)]
struct Opts {
    enabled: bool,
    position: String,
    background: Option<Paint>,
    title_color: Option<Paint>,
    body_color: Option<Paint>,
    footer_color: Option<Paint>,
    title_font: FontSpec,
    body_font: FontSpec,
    footer_font: FontSpec,
    title_spacing: f64,
    title_margin_bottom: f64,
    title_align: TextAlign,
    body_spacing: f64,
    body_align: TextAlign,
    footer_spacing: f64,
    footer_margin_top: f64,
    footer_align: TextAlign,
    padding: Padding,
    caret_padding: f64,
    caret_size: f64,
    corner_radius: Corners,
    box_width: f64,
    box_height: f64,
    multi_key_background: Option<Paint>,
    display_colors: bool,
    box_padding: f64,
    border_color: Option<Paint>,
    border_width: f64,
    use_point_style: bool,
    x_align: Option<XAlign>,
    y_align: Option<YAlign>,
    mode: Mode,
}

/// A laid-out tooltip, ready to draw.
#[derive(Debug, Clone)]
pub struct Tooltip {
    title: Vec<String>,
    before_body: Vec<String>,
    body: Vec<BodyItem>,
    after_body: Vec<String>,
    footer: Vec<String>,
    label_colors: Vec<LabelColor>,
    label_point_styles: Vec<(PointStyle, f64)>,
    label_text_colors: Vec<Option<Paint>>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    caret_x: f64,
    caret_y: f64,
    x_align: XAlign,
    y_align: YAlign,
    opts: Opts,
}

fn align(s: Option<&str>) -> TextAlign {
    match s {
        Some("center") => TextAlign::Center,
        Some("right") => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

/// `pushOrConcat(splitNewlines(v))`: a callback result as lines. Falsy values (`''`, `0`,
/// `false`, `null`) add nothing, as in JavaScript.
fn lines_of(v: Option<&Value>) -> Vec<String> {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => vec![],
        Some(Value::String(s)) if s.is_empty() => vec![],
        Some(Value::Number(n)) if n.as_f64() == Some(0.0) => vec![],
        Some(Value::Array(a)) => a.iter().map(js_to_string).collect(),
        Some(Value::String(s)) => s.split('\n').map(str::to_owned).collect(),
        Some(other) => vec![js_to_string(other)],
    }
}

impl Chart {
    /// The tooltip plugin's option scopes, plus `options.interaction` (its
    /// `additionalOptionScopes`).
    fn tooltip_scopes(&self) -> Scopes<'_> {
        self.plugin_scopes("tooltip")
            .then([&self.config.options["interaction"], &crate::defaults::DEFAULTS["interaction"]])
    }

    /// The interaction options the tooltip (and hover) uses: `plugins.tooltip.mode/intersect`,
    /// falling back to `options.interaction`, then Chart.js's defaults.
    pub fn tooltip_interaction(&self) -> InteractionOptions {
        let s = self.tooltip_scopes();
        let mut o = InteractionOptions::default();
        if let Some(m) = s.str("mode").and_then(Mode::parse) {
            o.mode = m;
        }
        o.intersect = s.bool_or("intersect", o.intersect);
        o.axis = s.str("axis").and_then(crate::interaction::Axis::parse);
        o.include_invisible = s.bool_or("includeInvisible", false);
        o
    }

    /// The `hover` options (`options.hover`, falling back to `options.interaction`).
    pub fn hover_interaction(&self) -> InteractionOptions {
        let s = Scopes::new([
            &self.config.options["hover"],
            &self.config.options["interaction"],
            &crate::defaults::DEFAULTS["interaction"],
        ]);
        let mut o = InteractionOptions::default();
        if let Some(m) = s.str("mode").and_then(Mode::parse) {
            o.mode = m;
        }
        o.intersect = s.bool_or("intersect", o.intersect);
        o.axis = s.str("axis").and_then(crate::interaction::Axis::parse);
        o
    }

    fn tooltip_opts(&self) -> Opts {
        let s = self.tooltip_scopes();
        let global = Scopes::new([&self.font_fallback]);
        let font = |k: &str| FontSpec::resolve(&s.child(k), &global);
        let body_font = font("bodyFont");
        let paint = |k: &str| s.get(k).and_then(|v| crate::gradient::paint(v, &self.chart_area));
        Opts {
            enabled: s.bool_or("enabled", true),
            position: s.str("position").unwrap_or("average").to_owned(),
            background: paint("backgroundColor"),
            title_color: paint("titleColor"),
            body_color: paint("bodyColor"),
            footer_color: paint("footerColor"),
            title_font: font("titleFont"),
            footer_font: font("footerFont"),
            title_spacing: s.num_or("titleSpacing", 2.0),
            title_margin_bottom: s.num_or("titleMarginBottom", 6.0),
            title_align: align(s.str("titleAlign")),
            body_spacing: s.num_or("bodySpacing", 2.0),
            body_align: align(s.str("bodyAlign")),
            footer_spacing: s.num_or("footerSpacing", 2.0),
            footer_margin_top: s.num_or("footerMarginTop", 6.0),
            footer_align: align(s.str("footerAlign")),
            padding: Padding::from_value(s.get("padding")),
            caret_padding: s.num_or("caretPadding", 2.0),
            caret_size: s.num_or("caretSize", 5.0),
            corner_radius: Corners::from_value(s.get("cornerRadius")),
            box_width: s.num("boxWidth").unwrap_or(body_font.size),
            box_height: s.num("boxHeight").unwrap_or(body_font.size),
            multi_key_background: paint("multiKeyBackground"),
            display_colors: s.bool_or("displayColors", true),
            box_padding: s.num_or("boxPadding", 0.0),
            border_color: paint("borderColor"),
            border_width: s.num_or("borderWidth", 0.0),
            use_point_style: s.bool_or("usePointStyle", false),
            x_align: match s.str("xAlign") {
                Some("left") => Some(XAlign::Left),
                Some("center") => Some(XAlign::Center),
                Some("right") => Some(XAlign::Right),
                _ => None,
            },
            y_align: match s.str("yAlign") {
                Some("top") => Some(YAlign::Top),
                Some("center") => Some(YAlign::Center),
                Some("bottom") => Some(YAlign::Bottom),
                _ => None,
            },
            mode: s.str("mode").and_then(Mode::parse).unwrap_or(Mode::Nearest),
            body_font,
        }
    }

    /// `getLabelAndValue(index)` for the dataset's controller type.
    fn label_and_value(&self, e: ActiveElement) -> Option<(String, String)> {
        let meta = self.metas.iter().find(|m| m.index == e.dataset)?;
        let p = meta.parsed.get(e.index)?;
        let labels = self.config.labels();
        let label_at = |i: usize| labels.get(i).map(|l| l.lines().join(",")).unwrap_or_default();
        Some(match meta.kind {
            ChartType::Pie | ChartType::Doughnut | ChartType::PolarArea => {
                (label_at(e.index), p.y.map(crate::format::format_default).unwrap_or_default())
            }
            ChartType::Radar => {
                let r = self.radial.as_ref()?;
                let label = r.point_labels.get(e.index).map(|l| l.lines().join(",")).unwrap_or_default();
                (label, p.y.map(crate::format::format_default).unwrap_or_default())
            }
            ChartType::Scatter | ChartType::Bubble => {
                let x = p.x.map(|v| self.scales[meta.x_scale].label_for_value(v)).unwrap_or_default();
                let y = p.y.map(|v| self.scales[meta.y_scale].label_for_value(v)).unwrap_or_default();
                let r = if meta.kind == ChartType::Bubble {
                    self.datasets[meta.index]["data"]
                        .get(e.index)
                        .and_then(|d| d.get("r"))
                        .and_then(as_num)
                        .map(|r| format!(", {}", crate::config::js_number_to_string(r)))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                (label_at(e.index), format!("({x}, {y}{r})"))
            }
            ChartType::Line | ChartType::Bar => {
                let (is, vs) = (meta.i_scale(), meta.v_scale());
                let iax = self.scales[is].axis;
                let vax = self.scales[vs].axis;
                let label = p.get(iax).map(|v| self.scales[is].label_for_value(v)).unwrap_or_default();
                let value = match p.custom {
                    Some(c) => format!(
                        "[{}, {}]",
                        crate::config::js_number_to_string(c.start),
                        crate::config::js_number_to_string(c.end)
                    ),
                    None => p.get(vax).map(|v| self.scales[vs].label_for_value(v)).unwrap_or_default(),
                };
                (label, value)
            }
        })
    }

    /// `controller.getStyle(index)`: the element's default-mode style, for the color box.
    fn tooltip_label_color(&self, e: ActiveElement) -> Option<(LabelColor, (PointStyle, f64))> {
        let meta = self.metas.iter().find(|m| m.index == e.dataset)?;
        let el = if !meta.bars.is_empty() {
            "bar"
        } else if !meta.arcs.is_empty() {
            "arc"
        } else {
            "point"
        };
        let s = self.element_scopes(meta.index, meta.kind, el);
        let i = Some(e.index);
        let bw = s.get_at("borderWidth", i);
        let border_width = match bw {
            Some(Value::Object(o)) => o.values().filter_map(as_num).fold(f64::MIN, f64::max),
            Some(v) => as_num(v).unwrap_or(0.0),
            None => 0.0,
        };
        Some((
            LabelColor {
                border: self.paint_at(&s, "borderColor", i),
                background: self.paint_at(&s, "backgroundColor", i),
                border_width,
                dash: s.num_list("borderDash"),
                dash_offset: s.num_or("borderDashOffset", 0.0),
                border_radius: Corners::default(),
            },
            (PointStyle::from_value(s.get_at("pointStyle", i)), s.num_at("rotation", i).unwrap_or(0.0)),
        ))
    }

    fn tooltip_item_json(&self, item: &TooltipItem) -> Value {
        let mut ctx = self.data_context(item.element.dataset, item.element.index);
        ctx["label"] = json!(item.label);
        ctx["formattedValue"] = json!(item.formatted_value);
        ctx["element"] = match tooltip_position(self, item.element) {
            Some((x, y)) => json!({ "x": x, "y": y }),
            None => Value::Null,
        };
        ctx
    }

    /// Call `callbacks.<name>` with `arg` if it's a function; `None` means "use the default".
    fn tooltip_callback(&self, name: &str, arg: Value) -> Option<Value> {
        let s = self.tooltip_scopes().child("callbacks");
        let id = s.fn_id(name)?;
        let script = s.script()?;
        let mut ctx = self.chart_context();
        ctx["type"] = json!("tooltip");
        ctx["__args"] = json!([arg]);
        script.call(&id, &ctx)
    }

    /// Lay out a tooltip for `active` elements (`event` is the pointer position, for the
    /// `nearest` positioner). `None` when there's nothing to show.
    pub fn layout_tooltip(
        &self,
        c: &mut dyn Canvas,
        active: &[ActiveElement],
        event: Option<(f64, f64)>,
    ) -> Option<Tooltip> {
        if active.is_empty() {
            return None;
        }
        let opts = self.tooltip_opts();
        // Positioner.
        let positions: Vec<(ActiveElement, (f64, f64))> = active
            .iter()
            .filter_map(|e| Some((*e, tooltip_position(self, *e)?)))
            .filter(|(_, p)| p.0.is_finite() && p.1.is_finite())
            .collect();
        if positions.is_empty() {
            return None;
        }
        let (caret_x, caret_y) = if opts.position == "nearest" {
            let ev = event.unwrap_or(positions[0].1);
            let nearest = positions
                .iter()
                .map(|(e, p)| (e, self.element_center(*e).unwrap_or(*p), p))
                .min_by(|a, b| {
                    let d = |q: (f64, f64)| (q.0 - ev.0).hypot(q.1 - ev.1);
                    d(a.1).partial_cmp(&d(b.1)).unwrap_or(std::cmp::Ordering::Equal)
                })?;
            *nearest.2
        } else {
            // average: distinct x values, all y values.
            let mut xs: Vec<f64> = Vec::new();
            for (_, p) in &positions {
                if !xs.contains(&p.0) {
                    xs.push(p.0);
                }
            }
            let x = xs.iter().sum::<f64>() / xs.len() as f64;
            let y = positions.iter().map(|(_, p)| p.1).sum::<f64>() / positions.len() as f64;
            (x, y)
        };

        // Items and content.
        let items: Vec<TooltipItem> = active
            .iter()
            .filter_map(|e| {
                let (label, formatted_value) = self.label_and_value(*e)?;
                Some(TooltipItem { element: *e, label, formatted_value })
            })
            .collect();
        let items_json = Value::Array(items.iter().map(|i| self.tooltip_item_json(i)).collect());
        let dataset_mode = opts.mode == Mode::Dataset;
        let labels = self.config.labels();
        let cb_lines = |name: &str, arg: &Value| lines_of(self.tooltip_callback(name, arg.clone()).as_ref());

        let mut title = cb_lines("beforeTitle", &items_json);
        title.extend(match self.tooltip_callback("title", items_json.clone()) {
            Some(v) => lines_of(Some(&v)),
            None => {
                let t = items.first().map(|item| {
                    if dataset_mode {
                        self.datasets[item.element.dataset]["label"].as_str().unwrap_or("").to_owned()
                    } else if !item.label.is_empty() {
                        item.label.clone()
                    } else {
                        labels.get(item.element.index).map(|l| l.lines().join(",")).unwrap_or_default()
                    }
                });
                lines_of(t.map(Value::String).as_ref())
            }
        });
        title.extend(cb_lines("afterTitle", &items_json));
        let before_body = cb_lines("beforeBody", &items_json);
        let mut body = Vec::new();
        let mut label_colors = Vec::new();
        let mut label_point_styles = Vec::new();
        let mut label_text_colors = Vec::new();
        for (item, json_item) in items.iter().zip(items_json.as_array().into_iter().flatten()) {
            let lines = match self.tooltip_callback("label", json_item.clone()) {
                Some(Value::Array(a)) => a.iter().map(js_to_string).collect(),
                Some(v) => vec![js_to_string(&v)],
                None => {
                    if dataset_mode {
                        vec![format!("{}: {}", item.label, item.formatted_value)]
                    } else {
                        let mut l =
                            self.datasets[item.element.dataset]["label"].as_str().unwrap_or("").to_owned();
                        if !l.is_empty() {
                            l.push_str(": ");
                        }
                        l.push_str(&item.formatted_value);
                        vec![l]
                    }
                }
            };
            body.push(BodyItem {
                before: cb_lines("beforeLabel", json_item),
                lines,
                after: cb_lines("afterLabel", json_item),
            });
            let (mut lc, ps) = self.tooltip_label_color(item.element)?;
            if let Some(Value::Object(o)) = self.tooltip_callback("labelColor", json_item.clone()) {
                let p = |k: &str| o.get(k).and_then(|v| crate::gradient::paint(v, &self.chart_area));
                lc.border = p("borderColor").or(lc.border);
                lc.background = p("backgroundColor").or(lc.background);
                lc.border_width = o.get("borderWidth").and_then(as_num).unwrap_or(lc.border_width);
                lc.border_radius = Corners::from_value(o.get("borderRadius"));
            }
            label_colors.push(lc);
            label_point_styles.push(ps);
            label_text_colors.push(
                self.tooltip_callback("labelTextColor", json_item.clone())
                    .and_then(|v| crate::gradient::paint(&v, &self.chart_area))
                    .or_else(|| opts.body_color.clone()),
            );
        }
        let after_body = cb_lines("afterBody", &items_json);
        let mut footer = cb_lines("beforeFooter", &items_json);
        footer.extend(cb_lines("footer", &items_json));
        footer.extend(cb_lines("afterFooter", &items_json));

        let mut t = Tooltip {
            title,
            before_body,
            body,
            after_body,
            footer,
            label_colors,
            label_point_styles,
            label_text_colors,
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            caret_x,
            caret_y,
            x_align: XAlign::Center,
            y_align: YAlign::Center,
            opts,
        };
        let (w, h) = t.size(c);
        t.width = w;
        t.height = h;
        // determineAlignment
        let o = &t.opts;
        let y_align = o.y_align.unwrap_or(if caret_y < h / 2.0 {
            YAlign::Top
        } else if caret_y > self.height - h / 2.0 {
            YAlign::Bottom
        } else {
            YAlign::Center
        });
        let x_align = o.x_align.unwrap_or_else(|| {
            let a = &self.chart_area;
            let mut xa = if y_align == YAlign::Center {
                if caret_x <= (a.left + a.right) / 2.0 { XAlign::Left } else { XAlign::Right }
            } else if caret_x <= w / 2.0 {
                XAlign::Left
            } else if caret_x >= self.width - w / 2.0 {
                XAlign::Right
            } else {
                XAlign::Center
            };
            let caret = o.caret_size + o.caret_padding;
            let does_not_fit = (xa == XAlign::Left && caret_x + w + caret > self.width)
                || (xa == XAlign::Right && caret_x - w - caret < 0.0);
            if does_not_fit {
                xa = XAlign::Center;
            }
            xa
        });
        // getBackgroundPoint
        let padding_and_size = o.caret_size + o.caret_padding;
        let r = o.corner_radius;
        let mut x = match x_align {
            XAlign::Right => caret_x - w,
            XAlign::Center => caret_x - w / 2.0,
            XAlign::Left => caret_x,
        };
        let y = match y_align {
            YAlign::Top => caret_y + padding_and_size,
            YAlign::Bottom => caret_y - (h + padding_and_size),
            YAlign::Center => caret_y - h / 2.0,
        };
        if y_align == YAlign::Center {
            match x_align {
                XAlign::Left => x += padding_and_size,
                XAlign::Right => x -= padding_and_size,
                XAlign::Center => {}
            }
        } else if x_align == XAlign::Left {
            x -= r.top_left.max(r.bottom_left) + o.caret_size;
        } else if x_align == XAlign::Right {
            x += r.top_right.max(r.bottom_right) + o.caret_size;
        }
        // _limitValue(v, 0, size): max(0, min(size, v)).
        t.x = x.min(self.width - w).max(0.0);
        t.y = y.min(self.height - h).max(0.0);
        t.x_align = x_align;
        t.y_align = y_align;
        Some(t)
    }

    /// An element's `getCenterPoint`.
    fn element_center(&self, e: ActiveElement) -> Option<(f64, f64)> {
        let meta = self.metas.iter().find(|m| m.index == e.dataset)?;
        if let Some(b) = meta.bars.get(e.index) {
            return Some(if b.horizontal {
                ((b.x + b.base) / 2.0, b.y)
            } else {
                (b.x, (b.y + b.base) / 2.0)
            });
        }
        if let Some(a) = meta.arcs.get(e.index) {
            return Some(arc_center(a));
        }
        meta.points.get(e.index).map(|p| (p.x, p.y))
    }
}

impl Tooltip {
    /// The box: `(x, y, width, height)` in CSS pixels.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        (self.x, self.y, self.width, self.height)
    }

    /// The point the caret indicates.
    pub fn caret_point(&self) -> (f64, f64) {
        (self.caret_x, self.caret_y)
    }

    /// `getTooltipSize`.
    fn size(&self, c: &mut dyn Canvas) -> (f64, f64) {
        let o = &self.opts;
        let mut height = o.padding.height();
        let mut width: f64 = 0.0;
        let title_lines = self.title.len() as f64;
        let footer_lines = self.footer.len() as f64;
        let body_items = self.body.len() as f64;
        let combined =
            self.body.iter().map(|b| b.before.len() + b.lines.len() + b.after.len()).sum::<usize>()
                + self.before_body.len()
                + self.after_body.len();
        let combined = combined as f64;
        if title_lines > 0.0 {
            height += title_lines * o.title_font.line_height
                + (title_lines - 1.0) * o.title_spacing
                + o.title_margin_bottom;
        }
        if combined > 0.0 {
            let body_lh = if o.display_colors {
                o.box_height.max(o.body_font.line_height)
            } else {
                o.body_font.line_height
            };
            height += body_items * body_lh
                + (combined - body_items) * o.body_font.line_height
                + (combined - 1.0) * o.body_spacing;
        }
        if footer_lines > 0.0 {
            height += o.footer_margin_top
                + footer_lines * o.footer_font.line_height
                + (footer_lines - 1.0) * o.footer_spacing;
        }
        let mut measure = |c: &mut dyn Canvas, line: &str, pad: f64| {
            width = width.max(f64::from(c.measure_text(line).width) + pad);
        };
        c.save();
        c.set_font(o.title_font.to_canvas());
        for l in &self.title {
            measure(c, l, 0.0);
        }
        c.set_font(o.body_font.to_canvas());
        for l in self.before_body.iter().chain(&self.after_body) {
            measure(c, l, 0.0);
        }
        let pad = if o.display_colors { o.box_width + 2.0 + o.box_padding } else { 0.0 };
        for b in &self.body {
            for l in b.before.iter().chain(&b.lines).chain(&b.after) {
                measure(c, l, pad);
            }
        }
        c.set_font(o.footer_font.to_canvas());
        for l in &self.footer {
            measure(c, l, 0.0);
        }
        c.restore();
        (width + o.padding.width(), height)
    }

    fn aligned_x(&self, align: TextAlign) -> f64 {
        match align {
            TextAlign::Center => self.x + self.width / 2.0,
            TextAlign::Right | TextAlign::End => self.x + self.width - self.opts.padding.right,
            TextAlign::Left | TextAlign::Start => self.x + self.opts.padding.left,
        }
    }

    /// `getCaretPosition`.
    fn caret(&self) -> [(f64, f64); 3] {
        let (o, r) = (&self.opts, self.opts.corner_radius);
        let (pt_x, pt_y, w, h, cs) = (self.x, self.y, self.width, self.height, o.caret_size);
        if self.y_align == YAlign::Center {
            let y2 = pt_y + h / 2.0;
            if self.x_align == XAlign::Left {
                let x1 = pt_x;
                [(x1, y2 + cs), (x1 - cs, y2), (x1, y2 - cs)]
            } else {
                let x1 = pt_x + w;
                [(x1, y2 - cs), (x1 + cs, y2), (x1, y2 + cs)]
            }
        } else {
            let x2 = match self.x_align {
                XAlign::Left => pt_x + r.top_left.max(r.bottom_left) + cs,
                XAlign::Right => pt_x + w - r.top_right.max(r.bottom_right) - cs,
                XAlign::Center => self.caret_x,
            };
            if self.y_align == YAlign::Top {
                let y1 = pt_y;
                [(x2 - cs, y1), (x2, y1 - cs), (x2 + cs, y1)]
            } else {
                let y1 = pt_y + h;
                [(x2 + cs, y1), (x2, y1 + cs), (x2 - cs, y1)]
            }
        }
    }

    /// Draw the tooltip (`Tooltip.draw`).
    pub fn draw(&self, c: &mut dyn Canvas) {
        let o = &self.opts;
        let has_content = !(self.title.is_empty()
            && self.before_body.is_empty()
            && self.body.is_empty()
            && self.after_body.is_empty()
            && self.footer.is_empty());
        if !o.enabled || !has_content {
            return;
        }
        c.save();
        self.draw_background(c);
        let mut pt = (self.x, self.y + o.padding.top);
        self.draw_title(c, &mut pt);
        self.draw_body(c, &mut pt);
        self.draw_footer(c, &mut pt);
        c.restore();
    }

    fn draw_background(&self, c: &mut dyn Canvas) {
        let o = &self.opts;
        let (x, y, w, h) = (self.x, self.y, self.width, self.height);
        let r = o.corner_radius;
        let f = |v: f64| v as f32;
        c.set_fill_style(o.background.clone().unwrap_or(Paint::Solid(Color::rgba(0.0, 0.0, 0.0, 0.8))));
        c.set_stroke_style(o.border_color.clone().unwrap_or(Paint::Solid(Color::TRANSPARENT)));
        c.set_line_width(o.border_width as f32);
        c.begin_path();
        c.move_to(f(x + r.top_left), f(y));
        let caret = |c: &mut dyn Canvas| {
            for (cx, cy) in self.caret() {
                c.line_to(f(cx), f(cy));
            }
        };
        if self.y_align == YAlign::Top {
            caret(c);
        }
        c.line_to(f(x + w - r.top_right), f(y));
        c.quadratic_curve_to(f(x + w), f(y), f(x + w), f(y + r.top_right));
        if self.y_align == YAlign::Center && self.x_align == XAlign::Right {
            caret(c);
        }
        c.line_to(f(x + w), f(y + h - r.bottom_right));
        c.quadratic_curve_to(f(x + w), f(y + h), f(x + w - r.bottom_right), f(y + h));
        if self.y_align == YAlign::Bottom {
            caret(c);
        }
        c.line_to(f(x + r.bottom_left), f(y + h));
        c.quadratic_curve_to(f(x), f(y + h), f(x), f(y + h - r.bottom_left));
        if self.y_align == YAlign::Center && self.x_align == XAlign::Left {
            caret(c);
        }
        c.line_to(f(x), f(y + r.top_left));
        c.quadratic_curve_to(f(x), f(y), f(x + r.top_left), f(y));
        c.close_path();
        c.fill();
        if o.border_width > 0.0 {
            c.stroke();
        }
    }

    fn draw_title(&self, c: &mut dyn Canvas, pt: &mut (f64, f64)) {
        let o = &self.opts;
        if self.title.is_empty() {
            return;
        }
        pt.0 = self.aligned_x(o.title_align);
        c.set_text_align(o.title_align);
        c.set_text_baseline(TextBaseline::Middle);
        c.set_fill_style(o.title_color.clone().unwrap_or(Paint::Solid(Color::WHITE)));
        c.set_font(o.title_font.to_canvas());
        let n = self.title.len();
        for (i, line) in self.title.iter().enumerate() {
            c.fill_text(line, pt.0 as f32, (pt.1 + o.title_font.line_height / 2.0) as f32, None);
            pt.1 += o.title_font.line_height + o.title_spacing;
            if i + 1 == n {
                pt.1 += o.title_margin_bottom - o.title_spacing;
            }
        }
    }

    fn draw_color_box(&self, c: &mut dyn Canvas, pt: (f64, f64), i: usize) {
        let o = &self.opts;
        let lc = &self.label_colors[i];
        let (bw, bh) = (o.box_width, o.box_height);
        let color_x = self.aligned_x(TextAlign::Left);
        let y_offset = if bh < o.body_font.line_height { (o.body_font.line_height - bh) / 2.0 } else { 0.0 };
        let color_y = pt.1 + y_offset;
        let multi = o.multi_key_background.clone().unwrap_or(Paint::Solid(Color::WHITE));
        let transparent = Paint::Solid(Color::TRANSPARENT);
        if o.use_point_style {
            let (style, rotation) = self.label_point_styles[i];
            let radius = bw.min(bh) / 2.0;
            let (cx, cy) = (color_x + bw / 2.0, color_y + bh / 2.0);
            c.set_stroke_style(multi.clone());
            c.set_fill_style(multi);
            c.set_line_width(1.0);
            draw_point(c, style, radius, rotation, 1.0, cx, cy, None);
            c.set_stroke_style(lc.border.clone().unwrap_or(transparent.clone()));
            c.set_fill_style(lc.background.clone().unwrap_or(transparent));
            draw_point(c, style, radius, rotation, 1.0, cx, cy, None);
        } else {
            let lw = if lc.border_width > 0.0 { lc.border_width } else { 1.0 };
            c.set_line_width(lw as f32);
            c.set_stroke_style(lc.border.clone().unwrap_or(transparent.clone()));
            c.set_line_dash(&lc.dash.iter().map(|d| *d as f32).collect::<Vec<_>>());
            c.set_line_dash_offset(lc.dash_offset as f32);
            c.set_line_join(LineJoin::Miter);
            let (outer_x, inner_x) = (color_x, color_x + 1.0);
            if lc.border_radius.any() {
                c.begin_path();
                c.set_fill_style(multi);
                rounded_rect_path(c, outer_x, color_y, bw, bh, lc.border_radius);
                c.fill();
                c.stroke();
                c.set_fill_style(lc.background.clone().unwrap_or(transparent));
                c.begin_path();
                rounded_rect_path(c, inner_x, color_y + 1.0, bw - 2.0, bh - 2.0, lc.border_radius);
                c.fill();
            } else {
                let f = |v: f64| v as f32;
                c.set_fill_style(multi);
                c.fill_rect(f(outer_x), f(color_y), f(bw), f(bh));
                c.stroke_rect(f(outer_x), f(color_y), f(bw), f(bh));
                c.set_fill_style(lc.background.clone().unwrap_or(transparent));
                c.fill_rect(f(inner_x), f(color_y + 1.0), f(bw - 2.0), f(bh - 2.0));
            }
            c.set_line_dash(&[]);
        }
    }

    fn draw_body(&self, c: &mut dyn Canvas, pt: &mut (f64, f64)) {
        let o = &self.opts;
        let font = &o.body_font;
        c.set_text_align(o.body_align);
        c.set_text_baseline(TextBaseline::Middle);
        c.set_font(font.to_canvas());
        pt.0 = self.aligned_x(o.body_align);
        let body_color = o.body_color.clone().unwrap_or(Paint::Solid(Color::WHITE));
        c.set_fill_style(body_color.clone());
        let mut line_height = font.line_height;
        let mut x_pad = 0.0;
        let fill_line = |c: &mut dyn Canvas, line: &str, pt: &mut (f64, f64), lh: f64, x_pad: f64| {
            c.fill_text(line, (pt.0 + x_pad) as f32, (pt.1 + lh / 2.0) as f32, None);
            pt.1 += lh + o.body_spacing;
        };
        for l in &self.before_body {
            fill_line(c, l, pt, line_height, x_pad);
        }
        if o.display_colors && o.body_align != TextAlign::Right {
            x_pad = if o.body_align == TextAlign::Center {
                o.box_width / 2.0 + o.box_padding
            } else {
                o.box_width + 2.0 + o.box_padding
            };
        }
        for (i, item) in self.body.iter().enumerate() {
            let text_color = self.label_text_colors[i].clone().unwrap_or_else(|| body_color.clone());
            c.set_fill_style(text_color.clone());
            for l in &item.before {
                fill_line(c, l, pt, line_height, x_pad);
            }
            if o.display_colors && !item.lines.is_empty() {
                c.save();
                self.draw_color_box(c, *pt, i);
                c.restore();
                c.set_fill_style(text_color);
                line_height = font.line_height.max(o.box_height);
            }
            for l in &item.lines {
                fill_line(c, l, pt, line_height, x_pad);
                line_height = font.line_height;
            }
            for l in &item.after {
                fill_line(c, l, pt, line_height, x_pad);
            }
        }
        line_height = font.line_height;
        for l in &self.after_body {
            fill_line(c, l, pt, line_height, 0.0);
        }
        pt.1 -= o.body_spacing;
    }

    fn draw_footer(&self, c: &mut dyn Canvas, pt: &mut (f64, f64)) {
        let o = &self.opts;
        if self.footer.is_empty() {
            return;
        }
        pt.0 = self.aligned_x(o.footer_align);
        pt.1 += o.footer_margin_top;
        c.set_text_align(o.footer_align);
        c.set_text_baseline(TextBaseline::Middle);
        c.set_fill_style(o.footer_color.clone().unwrap_or(Paint::Solid(Color::WHITE)));
        c.set_font(o.footer_font.to_canvas());
        for line in &self.footer {
            c.fill_text(line, pt.0 as f32, (pt.1 + o.footer_font.line_height / 2.0) as f32, None);
            pt.1 += o.footer_font.line_height + o.footer_spacing;
        }
    }
}
