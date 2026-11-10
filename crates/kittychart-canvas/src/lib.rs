//! A drawing API shaped like the HTML Canvas 2D context, limited to the subset Chart.js uses.
//!
//! Chart.js element and controller code is written against `CanvasRenderingContext2D`. Mirroring
//! that API (stateful path, `save`/`restore`, styles set before `fill`/`stroke`) lets KittyChart
//! port that logic close to line-for-line, which is the cheapest way to match Chart.js visually.
//!
//! Coordinates are logical ("CSS") pixels. Backends scale by the device pixel ratio internally,
//! as a browser canvas does when Chart.js sets `devicePixelRatio`.

mod color;
mod skia;
mod text;

pub use color::Color;
pub use skia::SkiaCanvas;
pub use text::{BUNDLED_FAMILY, Font, FontContext, TextAlign, TextBaseline, TextMetrics};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Path fill rule, as in `ctx.fill(fillRule)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    pub offset: f32,
    pub color: Color,
}

/// A fill or stroke style: `fillStyle` / `strokeStyle`.
#[derive(Debug, Clone, PartialEq)]
pub enum Paint {
    Solid(Color),
    /// `createLinearGradient(x0, y0, x1, y1)`
    Linear {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<ColorStop>,
    },
    /// `createRadialGradient(x0, y0, r0, x1, y1, r1)`
    Radial {
        x0: f32,
        y0: f32,
        r0: f32,
        x1: f32,
        y1: f32,
        r1: f32,
        stops: Vec<ColorStop>,
    },
    /// `createConicGradient(startAngle, x, y)`, with the angle in radians.
    Conic {
        start_angle: f32,
        x: f32,
        y: f32,
        stops: Vec<ColorStop>,
    },
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
    }
}

/// The Canvas 2D subset used by the chart engine. Dyn-compatible: plugins receive `&mut dyn Canvas`.
///
/// Path methods build a current path. `fill`, `stroke` and `clip` consume it without clearing it,
/// as on the web. Call `begin_path` to start over.
pub trait Canvas {
    /// Logical size.
    fn width(&self) -> f32;
    fn height(&self) -> f32;

    // State
    fn save(&mut self);
    fn restore(&mut self);
    fn translate(&mut self, x: f32, y: f32);
    fn scale(&mut self, x: f32, y: f32);
    fn rotate(&mut self, radians: f32);
    fn reset_transform(&mut self);

    // Styles
    fn set_fill_style(&mut self, paint: Paint);
    fn set_stroke_style(&mut self, paint: Paint);
    fn set_line_width(&mut self, w: f32);
    fn set_line_cap(&mut self, cap: LineCap);
    fn set_line_join(&mut self, join: LineJoin);
    fn set_miter_limit(&mut self, limit: f32);
    fn set_line_dash(&mut self, segments: &[f32]);
    fn set_line_dash_offset(&mut self, offset: f32);
    fn set_global_alpha(&mut self, alpha: f32);

    // Path
    fn begin_path(&mut self);
    fn move_to(&mut self, x: f32, y: f32);
    fn line_to(&mut self, x: f32, y: f32);
    fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32);
    fn bezier_curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, x: f32, y: f32);
    /// Canvas `arc`: angles in radians, clockwise from the +x axis (y points down).
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, counter_clockwise: bool);
    /// Canvas `ellipse`: radii `rx`/`ry`, rotated by `rotation` radians.
    #[allow(clippy::too_many_arguments)]
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, rotation: f32, start: f32, end: f32, ccw: bool);
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32);
    fn close_path(&mut self);

    // Painting
    fn fill(&mut self) {
        self.fill_with_rule(FillRule::NonZero);
    }
    fn fill_with_rule(&mut self, rule: FillRule);
    fn stroke(&mut self);
    /// Intersect the clip region with the current path.
    fn clip(&mut self);

    // Text
    /// `ctx.font = ...`
    fn set_font(&mut self, font: Font);
    fn set_text_align(&mut self, align: TextAlign);
    fn set_text_baseline(&mut self, baseline: TextBaseline);
    /// Draw a single line of text with the fill style, honoring transform, clip and alpha.
    /// With `max_width`, wider text is condensed horizontally to fit, as on the web.
    fn fill_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>);
    /// Outline a single line of text with the stroke style and line settings.
    fn stroke_text(&mut self, text: &str, x: f32, y: f32, max_width: Option<f32>);
    /// Measure text in the current font. Takes `&mut self` because shaping fills caches.
    fn measure_text(&mut self, text: &str) -> TextMetrics;

    fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.begin_path();
        self.rect(x, y, w, h);
        self.fill();
        self.begin_path();
    }
    fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.begin_path();
        self.rect(x, y, w, h);
        self.stroke();
        self.begin_path();
    }
    /// Clear a rectangle to transparent black, ignoring clip and alpha. This matches the web
    /// except that the web respects the clip.
    fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32);
}
