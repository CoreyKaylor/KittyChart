//! An SVG backend for the [`Canvas`] trait.
//!
//! It reuses [`SkiaCanvas`] for everything except painting: transforms, the drawing state, path
//! building and text shaping all run there, so geometry and text metrics are identical to the
//! PNG output. Painting emits SVG instead of pixels:
//!
//! - fills and strokes become `<path>` elements in device coordinates;
//! - clips become nested `<clipPath>`s;
//! - linear and radial gradients become `<linearGradient>`/`<radialGradient>` (SVG has no conic
//!   gradient, so conic fills are drawn as narrow wedges clipped to the shape);
//! - text becomes glyph outlines, so the file looks the same without the fonts installed.

use std::fmt::Write as _;

use tiny_skia as sk;

use crate::skia::DeviceStroke;
use crate::{
    Canvas, Color, ColorStop, FillRule, Font, LineCap, LineJoin, Paint, SkiaCanvas, TextAlign, TextBaseline,
    TextMetrics,
};

/// Wedges per turn when approximating a conic gradient.
const CONIC_STEPS: usize = 360;

/// Draws into an SVG document. Call [`SvgCanvas::finish`] for the markup.
#[derive(Debug)]
pub struct SvgCanvas {
    inner: SkiaCanvas,
    width: f32,
    height: f32,
    dpr: f32,
    defs: String,
    body: String,
    next_id: u32,
    /// The active clip (a `<clipPath>` id), and the clips saved with `save()`.
    clip: Option<String>,
    clip_stack: Vec<Option<String>>,
}

impl SvgCanvas {
    /// A `width × height` (CSS pixels) drawing. The SVG is `width·dpr × height·dpr` units, like
    /// the PNG at the same device pixel ratio.
    pub fn new(width: f32, height: f32, dpr: f32) -> SvgCanvas {
        let dpr = if dpr > 0.0 { dpr } else { 1.0 };
        SvgCanvas {
            // Geometry only: this canvas never rasterizes, so a 1×1 pixmap is enough.
            inner: SkiaCanvas::with_pixel_size(1, 1, dpr),
            width,
            height,
            dpr,
            defs: String::new(),
            body: String::new(),
            next_id: 0,
            clip: None,
            clip_stack: Vec::new(),
        }
    }

    /// Use these fonts for text (default: the bundled ones).
    pub fn with_fonts(mut self, fonts: std::sync::Arc<std::sync::Mutex<crate::FontContext>>) -> Self {
        self.inner = self.inner.with_fonts(fonts);
        self
    }

    /// Fill the whole drawing with `color` (a background).
    pub fn fill_background(&mut self, color: Color) {
        let (w, h) = (self.width * self.dpr, self.height * self.dpr);
        let _ = writeln!(
            self.body,
            r#"<rect width="{}" height="{}"{}/>"#,
            num(w),
            num(h),
            solid_attrs("fill", color, 1.0)
        );
    }

    /// The SVG document.
    pub fn finish(self) -> String {
        let (w, h) = (self.width * self.dpr, self.height * self.dpr);
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n",
            w = num(w),
            h = num(h)
        );
        if !self.defs.is_empty() {
            out.push_str("<defs>\n");
            out.push_str(&self.defs);
            out.push_str("</defs>\n");
        }
        out.push_str(&self.body);
        out.push_str("</svg>\n");
        out
    }

    fn id(&mut self, prefix: &str) -> String {
        self.next_id += 1;
        format!("{prefix}{}", self.next_id)
    }

    fn clip_attr(&self) -> String {
        self.clip.as_ref().map(|c| format!(r#" clip-path="url(#{c})""#)).unwrap_or_default()
    }

    /// Attributes painting `paint` as `fill` or `stroke`; `None` when it paints nothing. Conic
    /// gradients aren't expressible as attributes (see [`SvgCanvas::paint_conic`]).
    fn paint_attrs(&mut self, kind: &str, paint: &Paint) -> Option<String> {
        let alpha = self.inner.global_alpha();
        let t = self.inner.current_transform();
        let matrix = format!(
            "matrix({} {} {} {} {} {})",
            num(t[0]),
            num(t[1]),
            num(t[2]),
            num(t[3]),
            num(t[4]),
            num(t[5])
        );
        match paint {
            Paint::Solid(c) => (c.a * alpha > 0.0).then(|| solid_attrs(kind, *c, alpha)),
            Paint::Linear { x0, y0, x1, y1, stops } => {
                let id = self.id("g");
                let _ = writeln!(
                    self.defs,
                    r#"<linearGradient id="{id}" gradientUnits="userSpaceOnUse" gradientTransform="{matrix}" x1="{}" y1="{}" x2="{}" y2="{}">{}</linearGradient>"#,
                    num(*x0),
                    num(*y0),
                    num(*x1),
                    num(*y1),
                    stop_elems(stops, alpha)
                );
                Some(format!(r#" {kind}="url(#{id})""#))
            }
            Paint::Radial { x0, y0, r0, x1, y1, r1, stops } => {
                let id = self.id("g");
                let _ = writeln!(
                    self.defs,
                    r#"<radialGradient id="{id}" gradientUnits="userSpaceOnUse" gradientTransform="{matrix}" fx="{}" fy="{}" fr="{}" cx="{}" cy="{}" r="{}">{}</radialGradient>"#,
                    num(*x0),
                    num(*y0),
                    num(*r0),
                    num(*x1),
                    num(*y1),
                    num(*r1),
                    stop_elems(stops, alpha)
                );
                Some(format!(r#" {kind}="url(#{id})""#))
            }
            Paint::Conic { .. } => None,
        }
    }

    /// A conic gradient over `shape` (device space): a fan of wedges, each filled with the
    /// gradient's color at its angle, clipped to the shape.
    fn paint_conic(&mut self, shape_clip: &str, paint: &Paint) {
        let Paint::Conic { start_angle, x, y, stops } = paint else { return };
        let alpha = self.inner.global_alpha();
        let t = self.inner.current_transform();
        let (cx, cy) = (t[0] * x + t[2] * y + t[4], t[1] * x + t[3] * y + t[5]);
        // Far enough to cover the drawing from any center.
        let r = 2.0 * (self.width + self.height) * self.dpr + (cx.abs() + cy.abs());
        let rot = t[1].atan2(t[0]);
        // crispEdges: each pixel takes exactly one wedge's color, so thin wedges near the center
        // don't leave anti-aliased seams (moiré). The shape's edge is still smooth (the clip).
        let _ = write!(self.body, r#"<g clip-path="url(#{shape_clip})"><g shape-rendering="crispEdges">"#);
        for i in 0..CONIC_STEPS {
            let f0 = i as f32 / CONIC_STEPS as f32;
            let f1 = (i + 1) as f32 / CONIC_STEPS as f32;
            let color = sample(stops, (f0 + f1) / 2.0);
            let a0 = start_angle + rot + f0 * std::f32::consts::TAU;
            let a1 = start_angle + rot + f1 * std::f32::consts::TAU;
            let _ = write!(
                self.body,
                r#"<path d="M{} {}L{} {}L{} {}Z"{}/>"#,
                num(cx),
                num(cy),
                num(cx + r * a0.cos()),
                num(cy + r * a0.sin()),
                num(cx + r * a1.cos()),
                num(cy + r * a1.sin()),
                solid_attrs("fill", color, alpha)
            );
        }
        self.body.push_str("</g></g>\n");
    }

    fn fill_path(&mut self, path: &sk::Path, rule: FillRule, paint: &Paint) {
        let d = path_data(path);
        let rule_attr = if rule == FillRule::EvenOdd { r#" fill-rule="evenodd""# } else { "" };
        if matches!(paint, Paint::Conic { .. }) {
            let id = self.id("c");
            let clip = self.clip_attr();
            let clip_rule = if rule == FillRule::EvenOdd { r#" clip-rule="evenodd""# } else { "" };
            let _ = writeln!(self.defs, r#"<clipPath id="{id}"><path d="{d}"{clip_rule}{clip}/></clipPath>"#);
            self.paint_conic(&id, paint);
            return;
        }
        let Some(fill) = self.paint_attrs("fill", paint) else { return };
        let clip = self.clip_attr();
        let _ = writeln!(self.body, r#"<path d="{d}"{fill}{rule_attr}{clip}/>"#);
    }

    fn stroke_path(&mut self, path: &sk::Path) {
        let s: DeviceStroke = self.inner.device_stroke();
        if s.width <= 0.0 {
            return;
        }
        let paint = self.inner.stroke_paint().clone();
        let Some(stroke) = self.paint_attrs("stroke", &paint) else {
            if matches!(paint, Paint::Conic { .. }) {
                // Stroke a conic gradient: clip the wedges to the stroke's outline.
                let Some(outline) = stroke_outline(path, &s) else { return };
                let id = self.id("c");
                let _ = writeln!(
                    self.defs,
                    r#"<clipPath id="{id}"><path d="{}"{}/></clipPath>"#,
                    path_data(&outline),
                    self.clip_attr()
                );
                self.paint_conic(&id, &paint);
            }
            return;
        };
        let mut attrs = format!(r#" fill="none"{stroke} stroke-width="{}""#, num(s.width));
        match s.cap {
            LineCap::Butt => {}
            LineCap::Round => attrs.push_str(r#" stroke-linecap="round""#),
            LineCap::Square => attrs.push_str(r#" stroke-linecap="square""#),
        }
        match s.join {
            LineJoin::Miter => {
                if (s.miter_limit - 4.0).abs() > f32::EPSILON {
                    let _ = write!(attrs, r#" stroke-miterlimit="{}""#, num(s.miter_limit));
                }
            }
            LineJoin::Round => attrs.push_str(r#" stroke-linejoin="round""#),
            LineJoin::Bevel => attrs.push_str(r#" stroke-linejoin="bevel""#),
        }
        if !s.dash.is_empty() {
            let dash: Vec<String> = s.dash.iter().map(|d| num(*d)).collect();
            let _ = write!(attrs, r#" stroke-dasharray="{}""#, dash.join(" "));
            if s.dash_offset != 0.0 {
                let _ = write!(attrs, r#" stroke-dashoffset="{}""#, num(s.dash_offset));
            }
        }
        let clip = self.clip_attr();
        let _ = writeln!(self.body, r#"<path d="{}"{attrs}{clip}/>"#, path_data(path));
    }
}

fn stroke_outline(path: &sk::Path, s: &DeviceStroke) -> Option<sk::Path> {
    let stroke = sk::Stroke {
        width: s.width,
        miter_limit: s.miter_limit,
        line_cap: match s.cap {
            LineCap::Butt => sk::LineCap::Butt,
            LineCap::Round => sk::LineCap::Round,
            LineCap::Square => sk::LineCap::Square,
        },
        line_join: match s.join {
            LineJoin::Miter => sk::LineJoin::Miter,
            LineJoin::Round => sk::LineJoin::Round,
            LineJoin::Bevel => sk::LineJoin::Bevel,
        },
        dash: None,
    };
    path.stroke(&stroke, 1.0)
}

/// A number for SVG: at most 3 decimals, no trailing zeros.
fn num(v: f32) -> String {
    let s = format!("{:.3}", if v.abs() < 0.0005 { 0.0 } else { v });
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_owned() }
}

fn rgb(c: Color) -> String {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
}

fn solid_attrs(kind: &str, c: Color, alpha: f32) -> String {
    let a = (c.a * alpha).clamp(0.0, 1.0);
    if a >= 0.9995 {
        format!(r#" {kind}="{}""#, rgb(c))
    } else {
        format!(r#" {kind}="{}" {kind}-opacity="{}""#, rgb(c), num(a))
    }
}

fn stop_elems(stops: &[ColorStop], alpha: f32) -> String {
    stops
        .iter()
        .map(|s| {
            let a = (s.color.a * alpha).clamp(0.0, 1.0);
            let opacity = if a < 0.9995 { format!(r#" stop-opacity="{}""#, num(a)) } else { String::new() };
            format!(r#"<stop offset="{}" stop-color="{}"{opacity}/>"#, num(s.offset), rgb(s.color))
        })
        .collect()
}

/// The gradient's color at `t` (0..1), interpolating unpremultiplied like canvas gradients.
fn sample(stops: &[ColorStop], t: f32) -> Color {
    let Some(first) = stops.first() else { return Color::TRANSPARENT };
    if t <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.offset {
            let f = if b.offset > a.offset { (t - a.offset) / (b.offset - a.offset) } else { 1.0 };
            let l = |x: f32, y: f32| x + (y - x) * f;
            return Color {
                r: l(a.color.r, b.color.r),
                g: l(a.color.g, b.color.g),
                b: l(a.color.b, b.color.b),
                a: l(a.color.a, b.color.a),
            };
        }
    }
    stops.last().map_or(Color::TRANSPARENT, |s| s.color)
}

/// An SVG path `d` attribute for a device-space path.
fn path_data(path: &sk::Path) -> String {
    let mut d = String::new();
    let segs: Vec<sk::PathSegment> = path.segments().collect();
    // A trailing move-to (tiny-skia leaves one after closing a rect) draws nothing.
    let n = if matches!(segs.last(), Some(sk::PathSegment::MoveTo(_))) { segs.len() - 1 } else { segs.len() };
    for seg in segs.into_iter().take(n) {
        match seg {
            sk::PathSegment::MoveTo(p) => {
                let _ = write!(d, "M{} {}", num(p.x), num(p.y));
            }
            sk::PathSegment::LineTo(p) => {
                let _ = write!(d, "L{} {}", num(p.x), num(p.y));
            }
            sk::PathSegment::QuadTo(a, p) => {
                let _ = write!(d, "Q{} {} {} {}", num(a.x), num(a.y), num(p.x), num(p.y));
            }
            sk::PathSegment::CubicTo(a, b, p) => {
                let _ = write!(
                    d,
                    "C{} {} {} {} {} {}",
                    num(a.x),
                    num(a.y),
                    num(b.x),
                    num(b.y),
                    num(p.x),
                    num(p.y)
                );
            }
            sk::PathSegment::Close => d.push('Z'),
        }
    }
    d
}

impl Canvas for SvgCanvas {
    fn width(&self) -> f32 {
        self.width
    }
    fn height(&self) -> f32 {
        self.height
    }
    fn save(&mut self) {
        self.inner.save();
        self.clip_stack.push(self.clip.clone());
    }
    fn restore(&mut self) {
        self.inner.restore();
        if let Some(c) = self.clip_stack.pop() {
            self.clip = c;
        }
    }
    fn translate(&mut self, x: f32, y: f32) {
        self.inner.translate(x, y);
    }
    fn scale(&mut self, x: f32, y: f32) {
        self.inner.scale(x, y);
    }
    fn rotate(&mut self, angle: f32) {
        self.inner.rotate(angle);
    }
    fn reset_transform(&mut self) {
        self.inner.reset_transform();
    }
    fn set_fill_style(&mut self, paint: Paint) {
        self.inner.set_fill_style(paint);
    }
    fn set_stroke_style(&mut self, paint: Paint) {
        self.inner.set_stroke_style(paint);
    }
    fn set_line_width(&mut self, w: f32) {
        self.inner.set_line_width(w);
    }
    fn set_line_cap(&mut self, cap: LineCap) {
        self.inner.set_line_cap(cap);
    }
    fn set_line_join(&mut self, join: LineJoin) {
        self.inner.set_line_join(join);
    }
    fn set_miter_limit(&mut self, limit: f32) {
        self.inner.set_miter_limit(limit);
    }
    fn set_line_dash(&mut self, segments: &[f32]) {
        self.inner.set_line_dash(segments);
    }
    fn set_line_dash_offset(&mut self, offset: f32) {
        self.inner.set_line_dash_offset(offset);
    }
    fn set_global_alpha(&mut self, alpha: f32) {
        self.inner.set_global_alpha(alpha);
    }
    fn begin_path(&mut self) {
        self.inner.begin_path();
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.inner.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.inner.line_to(x, y);
    }
    fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        self.inner.quadratic_curve_to(cpx, cpy, x, y);
    }
    fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        self.inner.bezier_curve_to(cp1x, cp1y, cp2x, cp2y, x, y);
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, ccw: bool) {
        self.inner.arc(cx, cy, r, start, end, ccw);
    }
    #[allow(clippy::too_many_arguments)]
    fn ellipse(
        &mut self,
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
        rotation: f32,
        start: f32,
        end: f32,
        ccw: bool,
    ) {
        self.inner.ellipse(cx, cy, rx, ry, rotation, start, end, ccw);
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.inner.rect(x, y, w, h);
    }
    fn close_path(&mut self) {
        self.inner.close_path();
    }
    fn fill_with_rule(&mut self, rule: FillRule) {
        if let Some(path) = self.inner.device_path() {
            let paint = self.inner.fill_paint().clone();
            self.fill_path(&path, rule, &paint);
        }
    }
    fn stroke(&mut self) {
        if let Some(path) = self.inner.device_path() {
            self.stroke_path(&path);
        }
    }
    fn clip(&mut self) {
        let id = self.id("k");
        let d = self.inner.device_path().map(|p| path_data(&p)).unwrap_or_default();
        let parent = self.clip_attr();
        let _ = writeln!(self.defs, r#"<clipPath id="{id}"><path d="{d}"{parent}/></clipPath>"#);
        self.clip = Some(id);
    }
    fn set_font(&mut self, font: Font) {
        self.inner.set_font(font);
    }
    fn set_text_align(&mut self, align: TextAlign) {
        self.inner.set_text_align(align);
    }
    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.inner.set_text_baseline(baseline);
    }
    fn fill_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        if let Some(path) = self.inner.device_text_path(text, x, y, max_width) {
            let paint = self.inner.fill_paint().clone();
            self.fill_path(&path, FillRule::NonZero, &paint);
        }
    }
    fn stroke_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        if let Some(path) = self.inner.device_text_path(text, x, y, max_width) {
            self.stroke_path(&path);
        }
    }
    fn measure_text(&mut self, text: &str) -> TextMetrics {
        self.inner.measure_text(text)
    }
    fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // SVG can't erase. Clearing everything (what charts do before drawing) starts over.
        let t = self.inner.current_transform();
        let covers = x <= 0.0
            && y <= 0.0
            && (x + w) * t[0] >= self.width * self.dpr - 0.5
            && (y + h) * t[3] >= self.height * self.dpr - 0.5;
        if covers {
            self.body.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compact() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1.25), "1.25");
        assert_eq!(num(-0.0001), "0");
        assert_eq!(num(2.0004), "2");
    }

    #[test]
    fn draws_paths_clips_and_gradients() {
        let mut c = SvgCanvas::new(100.0, 50.0, 2.0);
        c.set_fill_style(Paint::Solid(Color::rgba8(255, 0, 0, 128)));
        c.rect(10.0, 10.0, 20.0, 10.0);
        c.fill();
        c.save();
        c.begin_path();
        c.rect(0.0, 0.0, 50.0, 50.0);
        c.clip();
        c.set_stroke_style(Paint::Linear {
            x0: 0.0,
            y0: 0.0,
            x1: 100.0,
            y1: 0.0,
            stops: vec![
                ColorStop { offset: 0.0, color: Color::BLACK },
                ColorStop { offset: 1.0, color: Color::WHITE },
            ],
        });
        c.begin_path();
        c.move_to(0.0, 25.0);
        c.line_to(100.0, 25.0);
        c.stroke();
        c.restore();
        let svg = c.finish();
        assert!(svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100""#));
        assert!(
            svg.contains(r##"<path d="M20 20L60 20L60 40L20 40Z" fill="#ff0000" fill-opacity="0.502"/>"##),
            "{svg}"
        );
        assert!(svg.contains("<linearGradient") && svg.contains(r#"clip-path="url(#k"#), "{svg}");
        assert!(svg.contains(r#"stroke-width="2""#), "line width scales with the dpr: {svg}");
    }
}
