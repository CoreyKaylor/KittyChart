//! Arc element (pie, doughnut, polar area): a port of Chart.js `element.arc.ts`.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use kittychart_canvas::{Canvas, Color, LineJoin, Paint};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpacingMode {
    Angular,
    Proportional,
    Parallel,
}

/// `borderRadius` for arcs: per corner.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ArcRadius {
    pub outer_start: f64,
    pub outer_end: f64,
    pub inner_start: f64,
    pub inner_end: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArcOptions {
    pub background_color: Option<Color>,
    pub border_color: Option<Color>,
    pub border_width: f64,
    pub border_align_inner: bool,
    pub border_join: Option<LineJoin>,
    pub border_dash: Vec<f64>,
    pub border_dash_offset: f64,
    pub border_radius: ArcRadius,
    pub offset: f64,
    pub spacing: f64,
    pub spacing_mode: SpacingMode,
    pub circular: bool,
}

#[derive(Debug, Clone)]
pub struct Arc {
    pub x: f64,
    pub y: f64,
    pub start_angle: f64,
    pub end_angle: f64,
    pub circumference: f64,
    pub inner_radius: f64,
    pub outer_radius: f64,
    pub options: ArcOptions,
}

impl Arc {
    pub fn draw(&self, c: &mut dyn Canvas) {
        let o = &self.options;
        let offset = o.offset / 4.0;
        let spacing = o.spacing / 2.0;
        let pixel_margin = if o.border_align_inner { 0.33 } else { 0.0 };
        let full_circles =
            if self.circumference > TAU { (self.circumference / TAU).floor() as usize } else { 0 };
        if self.circumference == 0.0 || self.inner_radius < 0.0 || self.outer_radius < 0.0 {
            return;
        }
        c.save();
        let half = (self.start_angle + self.end_angle) / 2.0;
        c.translate((half.cos() * offset) as f32, (half.sin() * offset) as f32);
        let fix = 1.0 - self.circumference.min(PI).sin();
        let radius_offset = offset * fix;
        c.set_fill_style(Paint::Solid(o.background_color.unwrap_or(Color::TRANSPARENT)));
        c.set_stroke_style(Paint::Solid(o.border_color.unwrap_or(Color::TRANSPARENT)));

        // drawArc
        let mut end = self.end_angle;
        if full_circles > 0 {
            self.path(c, radius_offset, spacing, end, pixel_margin);
            for _ in 0..full_circles {
                c.fill();
            }
            end = self.start_angle
                + if self.circumference % TAU == 0.0 { TAU } else { self.circumference % TAU };
        }
        self.path(c, radius_offset, spacing, end, pixel_margin);
        c.fill();

        // drawBorder
        if o.border_width > 0.0 {
            let dash: Vec<f32> = o.border_dash.iter().map(|d| *d as f32).collect();
            c.set_line_dash(&dash);
            c.set_line_dash_offset(o.border_dash_offset as f32);
            if o.border_align_inner {
                c.set_line_width((o.border_width * 2.0) as f32);
                c.set_line_join(o.border_join.unwrap_or(LineJoin::Round));
            } else {
                c.set_line_width(o.border_width as f32);
                c.set_line_join(o.border_join.unwrap_or(LineJoin::Bevel));
            }
            let mut end = self.end_angle;
            if full_circles > 0 {
                self.path(c, radius_offset, spacing, end, pixel_margin);
                for _ in 0..full_circles {
                    c.stroke();
                }
                end = self.start_angle
                    + if self.circumference % TAU == 0.0 { TAU } else { self.circumference % TAU };
            }
            if o.border_align_inner {
                self.clip_arc(c, end, pixel_margin);
            }
            if full_circles == 0 {
                self.path(c, radius_offset, spacing, end, pixel_margin);
                c.stroke();
            }
        }
        c.restore();
    }

    fn clip_arc(&self, c: &mut dyn Canvas, end: f64, pixel_margin: f64) {
        let f = |v: f64| v as f32;
        let mut margin = pixel_margin / self.outer_radius;
        c.begin_path();
        c.arc(
            f(self.x),
            f(self.y),
            f(self.outer_radius),
            f(self.start_angle - margin),
            f(end + margin),
            false,
        );
        if self.inner_radius > pixel_margin {
            margin = pixel_margin / self.inner_radius;
            c.arc(
                f(self.x),
                f(self.y),
                f(self.inner_radius),
                f(end + margin),
                f(self.start_angle - margin),
                true,
            );
        } else {
            c.arc(
                f(self.x),
                f(self.y),
                f(pixel_margin),
                f(end + FRAC_PI_2),
                f(self.start_angle - FRAC_PI_2),
                false,
            );
        }
        c.close_path();
        c.clip();
    }

    /// `pathArc`.
    fn path(&self, c: &mut dyn Canvas, offset: f64, spacing: f64, end: f64, pixel_margin: f64) {
        let f = |v: f64| v as f32;
        let (x, y, start) = (self.x, self.y, self.start_angle);
        let inner_r = self.inner_radius;
        let alpha = end - start;
        let outer_radius = (self.outer_radius + spacing + offset - pixel_margin).max(0.0);
        let mut inner_radius = if inner_r > 0.0 { inner_r + spacing + offset + pixel_margin } else { 0.0 };
        let (mut outer_spacing, mut inner_spacing) = (0.0, 0.0);
        let beta = if outer_radius > 0.0 {
            (alpha * outer_radius - offset / PI).max(0.001) / outer_radius
        } else {
            0.001
        };
        let angle_offset = (alpha - beta) / 2.0;
        if spacing != 0.0 {
            let no_spacing_inner = if inner_r > 0.0 { inner_r - spacing } else { 0.0 };
            let no_spacing_outer = if outer_radius > 0.0 { outer_radius - spacing } else { 0.0 };
            let avg = (no_spacing_inner + no_spacing_outer) / 2.0;
            let proportional = {
                let adjusted = if avg != 0.0 { (alpha * avg) / (avg + spacing) } else { alpha };
                (alpha - adjusted) / 2.0
            };
            let angular = if avg > 0.0 { (spacing / avg).min(1.0).asin() } else { 0.0 };
            let max_offset = (beta / 2.0 - 0.001).max(0.0);
            let max_offset_sin = max_offset.sin();
            match self.options.spacing_mode {
                SpacingMode::Parallel => {
                    if inner_radius == 0.0 && max_offset_sin > 0.0 {
                        inner_radius = (spacing / max_offset_sin).min((outer_radius - 0.001).max(0.0));
                    }
                    let max_parallel =
                        (if outer_radius > 0.0 { outer_radius * max_offset_sin } else { f64::INFINITY }).min(
                            if inner_radius > 0.0 { inner_radius * max_offset_sin } else { f64::INFINITY },
                        );
                    let ps = spacing.min(max_parallel);
                    outer_spacing = if outer_radius > 0.0 {
                        (ps / outer_radius).min(1.0).asin()
                    } else {
                        max_offset.min(angular)
                    };
                    inner_spacing =
                        if inner_radius > 0.0 { (ps / inner_radius).min(1.0).asin() } else { outer_spacing };
                }
                SpacingMode::Proportional => {
                    outer_spacing = max_offset.min(proportional);
                    inner_spacing = max_offset.min(proportional);
                }
                SpacingMode::Angular => {
                    outer_spacing = max_offset.min(angular);
                    inner_spacing = max_offset.min(angular);
                }
            }
        }
        let outer_start_angle = start + angle_offset + outer_spacing;
        let outer_end_angle = end - angle_offset - outer_spacing;
        let inner_start_angle = start + angle_offset + inner_spacing;
        let inner_end_angle = end - angle_offset - inner_spacing;
        let angle_delta = (outer_end_angle - outer_start_angle).min(inner_end_angle - inner_start_angle);

        // parseBorderRadius
        let r = self.options.border_radius;
        let half_thickness = (outer_radius - inner_radius) / 2.0;
        let inner_limit = half_thickness.min(angle_delta * inner_radius / 2.0);
        let outer_limit = |v: f64| {
            let arc_limit = (outer_radius - half_thickness.min(v)) * angle_delta / 2.0;
            v.clamp(0.0, half_thickness.min(arc_limit).max(0.0))
        };
        let outer_start = outer_limit(r.outer_start);
        let outer_end = outer_limit(r.outer_end);
        let inner_start = r.inner_start.clamp(0.0, inner_limit.max(0.0));
        let inner_end = r.inner_end.clamp(0.0, inner_limit.max(0.0));

        let os_r = outer_radius - outer_start;
        let oe_r = outer_radius - outer_end;
        let os_a = outer_start_angle + outer_start / os_r;
        let oe_a = outer_end_angle - outer_end / oe_r;
        let is_r = inner_radius + inner_start;
        let ie_r = inner_radius + inner_end;
        let is_a = inner_start_angle + inner_start / is_r;
        let ie_a = inner_end_angle - inner_end / ie_r;
        let xy = |r: f64, t: f64| (x + r * t.cos(), y + r * t.sin());

        c.begin_path();
        if self.options.circular {
            let mid = (os_a + oe_a) / 2.0;
            c.arc(f(x), f(y), f(outer_radius), f(os_a), f(mid), false);
            c.arc(f(x), f(y), f(outer_radius), f(mid), f(oe_a), false);
            if outer_end > 0.0 {
                let p = xy(oe_r, oe_a);
                c.arc(f(p.0), f(p.1), f(outer_end), f(oe_a), f(outer_end_angle + FRAC_PI_2), false);
            }
            let p4 = xy(ie_r, inner_end_angle);
            c.line_to(f(p4.0), f(p4.1));
            if inner_end > 0.0 {
                let p = xy(ie_r, ie_a);
                c.arc(f(p.0), f(p.1), f(inner_end), f(inner_end_angle + FRAC_PI_2), f(ie_a + PI), false);
            }
            let a1 = inner_end_angle - inner_end / inner_radius;
            let a2 = inner_start_angle + inner_start / inner_radius;
            let inner_mid = (a1 + a2) / 2.0;
            c.arc(f(x), f(y), f(inner_radius), f(a1), f(inner_mid), true);
            c.arc(f(x), f(y), f(inner_radius), f(inner_mid), f(a2), true);
            if inner_start > 0.0 {
                let p = xy(is_r, is_a);
                c.arc(f(p.0), f(p.1), f(inner_start), f(is_a + PI), f(inner_start_angle - FRAC_PI_2), false);
            }
            let p8 = xy(os_r, outer_start_angle);
            c.line_to(f(p8.0), f(p8.1));
            if outer_start > 0.0 {
                let p = xy(os_r, os_a);
                c.arc(f(p.0), f(p.1), f(outer_start), f(outer_start_angle - FRAC_PI_2), f(os_a), false);
            }
        } else {
            c.move_to(f(x), f(y));
            c.line_to(f(os_a.cos() * outer_radius + x), f(os_a.sin() * outer_radius + y));
            c.line_to(f(oe_a.cos() * outer_radius + x), f(oe_a.sin() * outer_radius + y));
        }
        c.close_path();
    }
}

/// `_normalizeAngle`.
pub fn normalize_angle(a: f64) -> f64 {
    (a % TAU + TAU) % TAU
}

/// `_angleBetween(angle, start, end, sameAngleIsFullCircle)`.
pub fn angle_between(angle: f64, start: f64, end: f64, same_is_full: bool) -> bool {
    let a = normalize_angle(angle);
    let s = normalize_angle(start);
    let e = normalize_angle(end);
    let to_start = normalize_angle(s - a);
    let to_end = normalize_angle(e - a);
    let start_to = normalize_angle(a - s);
    let end_to = normalize_angle(a - e);
    a == s || a == e || (same_is_full && s == e) || (to_start > to_end && start_to < end_to)
}
