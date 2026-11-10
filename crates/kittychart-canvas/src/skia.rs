//! tiny-skia raster backend.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::sync::{Arc, Mutex};

use tiny_skia as sk;

use crate::text::Shaped;
use crate::{
    Canvas, Color, ColorStop, FillRule, Font, FontContext, LineCap, LineJoin, Paint, TextAlign, TextBaseline,
    TextMetrics,
};

#[derive(Clone)]
struct State {
    transform: sk::Transform,
    fill: Paint,
    stroke: Paint,
    line_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
    dash: Vec<f32>,
    dash_offset: f32,
    alpha: f32,
    clip: Option<sk::Mask>,
    font: Font,
    text_align: TextAlign,
    text_baseline: TextBaseline,
}

/// Raster canvas backed by a [`tiny_skia::Pixmap`].
pub struct SkiaCanvas {
    pixmap: sk::Pixmap,
    dpr: f32,
    state: State,
    stack: Vec<State>,
    path: sk::PathBuilder,
    fonts: Arc<Mutex<FontContext>>,
    /// The current path's points are in device space. They are transformed as they're added,
    /// matching the web, where later transform changes don't affect segments already added.
    has_current_point: bool,
}

fn finite(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

impl std::fmt::Debug for SkiaCanvas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkiaCanvas")
            .field("px", &(self.pixmap.width(), self.pixmap.height()))
            .field("dpr", &self.dpr)
            .finish_non_exhaustive()
    }
}

impl SkiaCanvas {
    /// Create a canvas of `width × height` logical pixels at the given device pixel ratio.
    /// The backing pixmap is `ceil(width·dpr) × ceil(height·dpr)`.
    ///
    /// # Panics
    /// If the resulting pixel size is zero or overflows.
    pub fn new(width: f32, height: f32, dpr: f32) -> Self {
        let pw = (width * dpr).ceil() as u32;
        let ph = (height * dpr).ceil() as u32;
        let pixmap = sk::Pixmap::new(pw, ph).expect("canvas size must be non-zero");
        let base = sk::Transform::from_scale(dpr, dpr);
        Self {
            pixmap,
            dpr,
            state: State {
                transform: base,
                fill: Paint::Solid(Color::BLACK),
                stroke: Paint::Solid(Color::BLACK),
                line_width: 1.0,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
                miter_limit: 10.0,
                dash: Vec::new(),
                dash_offset: 0.0,
                alpha: 1.0,
                clip: None,
                font: Font::default(),
                text_align: TextAlign::default(),
                text_baseline: TextBaseline::default(),
            },
            stack: Vec::new(),
            path: sk::PathBuilder::new(),
            fonts: FontContext::shared_bundled(),
            has_current_point: false,
        }
    }

    /// Use a specific font context, for example one with system fonts. The default is the
    /// shared, bundled-only context, which renders identically on every machine.
    pub fn with_fonts(mut self, fonts: Arc<Mutex<FontContext>>) -> Self {
        self.fonts = fonts;
        self
    }

    /// A canvas with an exact device-pixel size; the logical size is `pixels / dpr`. Use it
    /// when the bitmap must match a target exactly (such as a whole number of terminal cells).
    ///
    /// # Panics
    /// If a dimension is zero or `dpr` isn't positive.
    pub fn with_pixel_size(pixel_width: u32, pixel_height: u32, dpr: f32) -> Self {
        assert!(dpr > 0.0, "device pixel ratio must be positive");
        let mut c = Self::new(1.0, 1.0, 1.0);
        c.pixmap = sk::Pixmap::new(pixel_width, pixel_height).expect("canvas size must be non-zero");
        c.dpr = dpr;
        c.state.transform = sk::Transform::from_scale(dpr, dpr);
        c
    }

    /// The current transform as `[sx, ky, kx, sy, tx, ty]` (device pixels).
    pub fn current_transform(&self) -> [f32; 6] {
        let t = self.state.transform;
        [t.sx, t.ky, t.kx, t.sy, t.tx, t.ty]
    }

    /// The current fill style.
    pub fn fill_paint(&self) -> &Paint {
        &self.state.fill
    }

    pub fn stroke_paint(&self) -> &Paint {
        &self.state.stroke
    }

    pub fn global_alpha(&self) -> f32 {
        self.state.alpha
    }

    pub fn current_text_align(&self) -> TextAlign {
        self.state.text_align
    }

    pub fn current_text_baseline(&self) -> TextBaseline {
        self.state.text_baseline
    }

    pub fn current_font(&self) -> &Font {
        &self.state.font
    }

    pub fn pixel_width(&self) -> u32 {
        self.pixmap.width()
    }
    pub fn pixel_height(&self) -> u32 {
        self.pixmap.height()
    }
    pub fn device_pixel_ratio(&self) -> f32 {
        self.dpr
    }

    /// Fill the whole canvas, ignoring transform and clip (a background).
    pub fn fill_background(&mut self, color: Color) {
        self.pixmap.fill(color.to_skia(1.0));
    }

    /// Draw a non-premultiplied RGBA8 image scaled into the device-pixel rectangle `(x, y, w, h)`,
    /// ignoring the current transform but respecting clip and global alpha.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_rgba_device(&mut self, x: f32, y: f32, w: f32, h: f32, rgba: &[u8], iw: u32, ih: u32) {
        let Some(size) = sk::IntSize::from_wh(iw, ih) else { return };
        let mut data = Vec::with_capacity(rgba.len());
        for p in rgba.as_chunks::<4>().0 {
            let a = u16::from(p[3]);
            let m = |c: u8| ((u16::from(c) * a + 127) / 255) as u8;
            data.extend_from_slice(&[m(p[0]), m(p[1]), m(p[2]), p[3]]);
        }
        let Some(img) = sk::Pixmap::from_vec(data, size) else { return };
        let ts = sk::Transform::from_row(w / iw as f32, 0.0, 0.0, h / ih as f32, x, y);
        let paint = sk::PixmapPaint {
            opacity: self.state.alpha,
            quality: if (w - iw as f32).abs() < 0.5 && (h - ih as f32).abs() < 0.5 {
                sk::FilterQuality::Nearest
            } else {
                sk::FilterQuality::Bicubic
            },
            ..sk::PixmapPaint::default()
        };
        self.pixmap.draw_pixmap(0, 0, img.as_ref(), &paint, ts, self.state.clip.as_ref());
    }

    /// Non-premultiplied RGBA8 pixels, row-major. This is what terminals and PNG expect.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixmap.data().len());
        for p in self.pixmap.pixels() {
            let c = p.demultiply();
            out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        out
    }

    /// Device-pixel width and height of the current path's bounds.
    pub fn current_path_size(&self) -> Option<(f32, f32)> {
        self.current_path().map(|p| (p.bounds().width(), p.bounds().height()))
    }

    /// Clear the pixels inside the current path (respecting the clip), like
    /// `globalCompositeOperation = "destination-out"`.
    pub fn erase_current_path(&mut self) {
        let Some(path) = self.current_path() else { return };
        let paint = sk::Paint {
            shader: sk::Shader::SolidColor(sk::Color::BLACK),
            blend_mode: sk::BlendMode::DestinationOut,
            anti_alias: true,
            ..sk::Paint::default()
        };
        self.pixmap.fill_path(
            &path,
            &paint,
            sk::FillRule::Winding,
            sk::Transform::identity(),
            self.state.clip.as_ref(),
        );
    }

    fn device_point(&self, x: f32, y: f32) -> (f32, f32) {
        let mut p = [sk::Point::from_xy(x, y)];
        self.state.transform.map_points(&mut p);
        (p[0].x, p[0].y)
    }

    fn current_path(&self) -> Option<sk::Path> {
        self.path.clone().finish()
    }

    fn shader(&self, paint: &Paint) -> Option<sk::Shader<'static>> {
        let a = self.state.alpha;
        let stops = |s: &[ColorStop]| -> Vec<sk::GradientStop> {
            s.iter().map(|s| sk::GradientStop::new(s.offset, s.color.to_skia(a))).collect()
        };
        // Gradients are defined in user space at creation time; the path is already in device
        // space, so the gradient gets the current transform.
        let t = self.state.transform;
        match paint {
            Paint::Solid(c) => Some(sk::Shader::SolidColor(c.to_skia(a))),
            Paint::Linear { x0, y0, x1, y1, stops: s } => sk::LinearGradient::new(
                sk::Point::from_xy(*x0, *y0),
                sk::Point::from_xy(*x1, *y1),
                stops(s),
                sk::SpreadMode::Pad,
                t,
            ),
            Paint::Radial { x0, y0, r0, x1, y1, r1, stops: s } => sk::RadialGradient::new(
                sk::Point::from_xy(*x0, *y0),
                *r0,
                sk::Point::from_xy(*x1, *y1),
                *r1,
                stops(s),
                sk::SpreadMode::Pad,
                t,
            ),
            Paint::Conic { start_angle, x, y, stops: s } => sk::SweepGradient::new(
                sk::Point::from_xy(*x, *y),
                start_angle.to_degrees(),
                start_angle.to_degrees() + 360.0,
                stops(s),
                sk::SpreadMode::Pad,
                t,
            ),
        }
    }

    fn shape(&self, text: &str) -> Shaped {
        self.fonts.lock().unwrap_or_else(std::sync::PoisonError::into_inner).shape(text, &self.state.font)
    }

    /// Text as one device-space path: glyph outlines placed per align and baseline, then mapped
    /// through the current transform.
    fn text_path(&self, text: &str, x: f32, y: f32, max_width: Option<f32>) -> Option<sk::Path> {
        let shaped = self.shape(text);
        // maxWidth condenses horizontally around the anchor, as browsers do.
        let k = match max_width {
            Some(m) if m > 0.0 && shaped.width > m => m / shaped.width,
            Some(m) if m <= 0.0 => return None,
            _ => 1.0,
        };
        let ox = x + shaped.align_dx(self.state.text_align) * k;
        let mut oy = y + shaped.baseline_dy(self.state.text_baseline);
        // Like Chromium/Skia, snap the baseline of axis-aligned text to a whole device pixel
        // (glyphs get subpixel positioning horizontally only). This keeps horizontal stems crisp.
        let t = self.state.transform;
        if t.kx == 0.0 && t.ky == 0.0 && t.sy > 0.0 {
            let device_y = t.sy * oy + t.ty;
            oy += (device_y.round() - device_y) / t.sy;
        }
        let mut fonts = self.fonts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut pb = sk::PathBuilder::new();
        let skew = sk::Transform::from_row(1.0, 0.0, -(14f32.to_radians().tan()), 1.0, 0.0, 0.0);
        for g in &shaped.glyphs {
            let Some((outline, upem)) = fonts.outline(g) else { continue };
            let s = g.size / upem;
            // Font units are y-up. Flip, scale to px, optionally skew, then place on the pen.
            let mut glyph = sk::Transform::from_scale(s, -s);
            if g.fake_italic {
                glyph = glyph.post_concat(skew);
            }
            let t = glyph
                .post_scale(k, 1.0)
                .post_translate(ox + g.x * k, oy + g.y)
                .post_concat(self.state.transform);
            if let Some(p) = outline.transform(t) {
                pb.push_path(&p);
            }
        }
        pb.finish()
    }

    fn sk_paint(&self, paint: &Paint) -> Option<sk::Paint<'static>> {
        Some(sk::Paint { shader: self.shader(paint)?, anti_alias: true, ..sk::Paint::default() })
    }
}

impl Canvas for SkiaCanvas {
    fn width(&self) -> f32 {
        self.pixmap.width() as f32 / self.dpr
    }
    fn height(&self) -> f32 {
        self.pixmap.height() as f32 / self.dpr
    }

    fn save(&mut self) {
        self.stack.push(self.state.clone());
    }
    fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }
    fn translate(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.pre_translate(x, y);
    }
    fn scale(&mut self, x: f32, y: f32) {
        self.state.transform = self.state.transform.pre_scale(x, y);
    }
    fn rotate(&mut self, radians: f32) {
        self.state.transform =
            self.state.transform.pre_concat(sk::Transform::from_rotate(radians.to_degrees()));
    }
    fn reset_transform(&mut self) {
        self.state.transform = sk::Transform::from_scale(self.dpr, self.dpr);
    }

    fn set_fill_style(&mut self, paint: Paint) {
        self.state.fill = paint;
    }
    fn set_stroke_style(&mut self, paint: Paint) {
        self.state.stroke = paint;
    }
    fn set_line_width(&mut self, w: f32) {
        if w.is_finite() && w > 0.0 {
            self.state.line_width = w;
        }
    }
    fn set_line_cap(&mut self, cap: LineCap) {
        self.state.cap = cap;
    }
    fn set_line_join(&mut self, join: LineJoin) {
        self.state.join = join;
    }
    fn set_miter_limit(&mut self, limit: f32) {
        if limit.is_finite() && limit > 0.0 {
            self.state.miter_limit = limit;
        }
    }
    fn set_line_dash(&mut self, segments: &[f32]) {
        if segments.iter().any(|s| !s.is_finite() || *s < 0.0) {
            return; // the web ignores invalid dash lists
        }
        // The web repeats odd-length lists to make them even.
        self.state.dash = if segments.len() % 2 == 1 { segments.repeat(2) } else { segments.to_vec() };
    }
    fn set_line_dash_offset(&mut self, offset: f32) {
        self.state.dash_offset = offset;
    }
    fn set_global_alpha(&mut self, alpha: f32) {
        if (0.0..=1.0).contains(&alpha) {
            self.state.alpha = alpha;
        }
    }

    fn begin_path(&mut self) {
        self.path.clear();
        self.has_current_point = false;
    }
    fn move_to(&mut self, x: f32, y: f32) {
        if !finite(&[x, y]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        let (x, y) = self.device_point(x, y);
        self.path.move_to(x, y);
        self.has_current_point = true;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if !finite(&[x, y]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        let (x, y) = self.device_point(x, y);
        if self.has_current_point {
            self.path.line_to(x, y);
        } else {
            self.path.move_to(x, y);
            self.has_current_point = true;
        }
    }
    fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        if !finite(&[cpx, cpy, x, y]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        if !self.has_current_point {
            self.move_to(cpx, cpy);
        }
        let (cx, cy) = self.device_point(cpx, cpy);
        let (x, y) = self.device_point(x, y);
        self.path.quad_to(cx, cy, x, y);
    }
    fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32) {
        if !finite(&[cp1x, cp1y, cp2x, cp2y, x, y]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        if !self.has_current_point {
            self.move_to(cp1x, cp1y);
        }
        let (ax, ay) = self.device_point(cp1x, cp1y);
        let (bx, by) = self.device_point(cp2x, cp2y);
        let (x, y) = self.device_point(x, y);
        self.path.cubic_to(ax, ay, bx, by, x, y);
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, ccw: bool) {
        if !finite(&[cx, cy, r, start, end]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        if r < 0.0 || !r.is_finite() {
            return;
        }
        // Normalize the sweep the way the HTML spec does.
        let mut sweep = end - start;
        if !ccw {
            if sweep >= TAU {
                sweep = TAU;
            } else {
                sweep = sweep.rem_euclid(TAU);
                if sweep == 0.0 && end != start {
                    sweep = TAU;
                }
            }
        } else if -sweep >= TAU {
            sweep = -TAU;
        } else {
            sweep = -((-sweep).rem_euclid(TAU));
            if sweep == 0.0 && end != start {
                sweep = -TAU;
            }
        }

        let (sx, sy) = (cx + r * start.cos(), cy + r * start.sin());
        self.line_to(sx, sy); // connects from the current point, or moves if there is none

        // Cubic approximation, one segment per ≤ 90°.
        let segments = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / segments as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        let mut a0 = start;
        for _ in 0..segments {
            let a1 = a0 + step;
            let (c0, s0) = (a0.cos(), a0.sin());
            let (c1, s1) = (a1.cos(), a1.sin());
            self.bezier_curve_to(
                cx + r * (c0 - k * s0),
                cy + r * (s0 + k * c0),
                cx + r * (c1 + k * s1),
                cy + r * (s1 - k * c1),
                cx + r * c1,
                cy + r * s1,
            );
            a0 = a1;
        }
    }
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
        if rx < 0.0 || ry < 0.0 {
            return;
        }
        // Draw a unit-circle arc under a temporary transform. Points are stored in device space,
        // so restoring the transform afterwards leaves the geometry in place.
        let saved = self.state.transform;
        self.state.transform = saved
            .pre_translate(cx, cy)
            .pre_concat(sk::Transform::from_rotate(rotation.to_degrees()))
            .pre_scale(rx.max(1e-6), ry.max(1e-6));
        self.arc(0.0, 0.0, 1.0, start, end, ccw);
        self.state.transform = saved;
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        if !finite(&[x, y, w, h]) {
            return; // Canvas spec: non-finite arguments make the call a no-op
        }
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
        self.move_to(x, y);
    }
    fn close_path(&mut self) {
        self.path.close();
    }

    fn fill_with_rule(&mut self, rule: FillRule) {
        if let Some(path) = self.current_path() {
            self.fill_device_path(&path, rule);
        }
    }

    fn stroke(&mut self) {
        if let Some(path) = self.current_path() {
            self.stroke_device_path(&path);
        }
    }

    fn clip(&mut self) {
        let Some(path) = self.current_path() else {
            // Clipping to an empty path clips everything.
            let mut m = sk::Mask::new(self.pixmap.width(), self.pixmap.height()).unwrap();
            m.clear();
            self.state.clip = Some(m);
            return;
        };
        match &mut self.state.clip {
            Some(mask) => mask.intersect_path(&path, sk::FillRule::Winding, true, sk::Transform::identity()),
            None => {
                let mut m = sk::Mask::new(self.pixmap.width(), self.pixmap.height()).unwrap();
                m.fill_path(&path, sk::FillRule::Winding, true, sk::Transform::identity());
                self.state.clip = Some(m);
            }
        }
    }

    fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let mut pb = sk::PathBuilder::new();
        if let Some(r) = sk::Rect::from_xywh(x, y, w, h) {
            pb.push_rect(r);
        }
        let Some(path) = pb.finish() else { return };
        let paint = sk::Paint {
            shader: sk::Shader::SolidColor(sk::Color::TRANSPARENT),
            blend_mode: sk::BlendMode::Clear,
            anti_alias: false,
            ..sk::Paint::default()
        };
        self.pixmap.fill_path(&path, &paint, sk::FillRule::Winding, self.state.transform, None);
    }

    fn set_font(&mut self, font: Font) {
        self.state.font = font;
    }
    fn set_text_align(&mut self, align: TextAlign) {
        self.state.text_align = align;
    }
    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.state.text_baseline = baseline;
    }
    fn fill_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        if let Some(p) = self.text_path(text, x, y, max_width) {
            self.fill_device_path(&p, FillRule::NonZero);
        }
    }
    fn stroke_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>) {
        if let Some(p) = self.text_path(text, x, y, max_width) {
            self.stroke_device_path(&p);
        }
    }
    fn measure_text(&mut self, text: &str) -> TextMetrics {
        let shaped = self.shape(text);
        let dx = shaped.align_dx(self.state.text_align);
        let dy = shaped.baseline_dy(self.state.text_baseline);
        // Ink bounds in user space (y-down), relative to the drawing point.
        let (mut l, mut t, mut r, mut b) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        {
            let mut fonts = self.fonts.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            for g in &shaped.glyphs {
                let Some((outline, upem)) = fonts.outline(g) else { continue };
                let s = g.size / upem;
                let bb = outline.bounds(); // font units, y-up
                l = l.min(dx + g.x + bb.left() * s);
                r = r.max(dx + g.x + bb.right() * s);
                t = t.min(dy + g.y - bb.bottom() * s);
                b = b.max(dy + g.y - bb.top() * s);
            }
        }
        let ink = l <= r;
        TextMetrics {
            width: shaped.width,
            actual_bounding_box_left: if ink { -l } else { 0.0 },
            actual_bounding_box_right: if ink { r } else { 0.0 },
            actual_bounding_box_ascent: if ink { -t } else { 0.0 },
            actual_bounding_box_descent: if ink { b } else { 0.0 },
            font_bounding_box_ascent: shaped.ascent - dy,
            font_bounding_box_descent: shaped.descent + dy,
        }
    }
}

impl SkiaCanvas {
    fn fill_device_path(&mut self, path: &sk::Path, rule: FillRule) {
        let Some(paint) = self.sk_paint(&self.state.fill) else { return };
        let rule = match rule {
            FillRule::NonZero => sk::FillRule::Winding,
            FillRule::EvenOdd => sk::FillRule::EvenOdd,
        };
        self.pixmap.fill_path(path, &paint, rule, sk::Transform::identity(), self.state.clip.as_ref());
    }

    fn stroke_device_path(&mut self, path: &sk::Path) {
        let Some(paint) = self.sk_paint(&self.state.stroke) else { return };
        // Paths are in device space, so scale stroke geometry by the transform's scale.
        // Chart.js never strokes under non-uniform scale, so the mean scale is exact in practice.
        let t = self.state.transform;
        let s = ((t.sx * t.sx + t.ky * t.ky).sqrt() + (t.kx * t.kx + t.sy * t.sy).sqrt()) / 2.0;
        let dash = if self.state.dash.iter().any(|d| *d > 0.0) {
            sk::StrokeDash::new(self.state.dash.iter().map(|d| d * s).collect(), self.state.dash_offset * s)
        } else {
            None
        };
        let stroke = sk::Stroke {
            width: self.state.line_width * s,
            miter_limit: self.state.miter_limit,
            line_cap: match self.state.cap {
                LineCap::Butt => sk::LineCap::Butt,
                LineCap::Round => sk::LineCap::Round,
                LineCap::Square => sk::LineCap::Square,
            },
            line_join: match self.state.join {
                LineJoin::Miter => sk::LineJoin::Miter,
                LineJoin::Round => sk::LineJoin::Round,
                LineJoin::Bevel => sk::LineJoin::Bevel,
            },
            dash,
        };
        self.pixmap.stroke_path(path, &paint, &stroke, sk::Transform::identity(), self.state.clip.as_ref());
    }
}
