//! Animations: Chart.js's easing functions and interpolators, and transitions between the
//! geometry a chart showed and the geometry it should show.
//!
//! Chart.js animates element properties: every element tweens from what is on screen to its
//! new target whenever the chart updates (new data, visibility changes, hover). Elements that
//! appear start from the controller's "reset" geometry (bars grow from their base, points
//! rise from the axis, doughnut arcs sweep from the start angle). KittyChart does the same
//! with snapshots of the drawn elements, driven by an explicit clock ([`crate::Chart::set_time`]),
//! so it stays deterministic and testable. Without a clock nothing animates.

use kittychart_canvas::{Color, Paint};

use crate::arc::Arc;
use crate::element::{Bar, Point};

/// `helpers.easing.effects`.
pub fn easing(name: &str) -> fn(f64) -> f64 {
    use std::f64::consts::{FRAC_PI_2 as HALF_PI, PI, TAU};
    fn at_edge(t: f64) -> bool {
        t == 0.0 || t == 1.0
    }
    fn elastic_in(t: f64, s: f64, p: f64) -> f64 {
        let t = t - 1.0;
        -(2f64.powf(10.0 * t) * ((t - s) * TAU / p).sin())
    }
    fn elastic_out(t: f64, s: f64, p: f64) -> f64 {
        2f64.powf(-10.0 * t) * ((t - s) * TAU / p).sin() + 1.0
    }
    fn out_bounce(t: f64) -> f64 {
        let (m, d) = (7.5625, 2.75);
        if t < 1.0 / d {
            m * t * t
        } else if t < 2.0 / d {
            let t = t - 1.5 / d;
            m * t * t + 0.75
        } else if t < 2.5 / d {
            let t = t - 2.25 / d;
            m * t * t + 0.9375
        } else {
            let t = t - 2.625 / d;
            m * t * t + 0.984375
        }
    }
    match name {
        "linear" => |t| t,
        "easeInQuad" => |t| t * t,
        "easeOutQuad" => |t| -t * (t - 2.0),
        "easeInOutQuad" => |t| {
            let t = t / 0.5;
            if t < 1.0 { 0.5 * t * t } else { -0.5 * ((t - 1.0) * (t - 3.0) - 1.0) }
        },
        "easeInCubic" => |t| t * t * t,
        "easeOutCubic" => |t| {
            let t = t - 1.0;
            t * t * t + 1.0
        },
        "easeInOutCubic" => |t| {
            let t = t / 0.5;
            if t < 1.0 {
                0.5 * t * t * t
            } else {
                let t = t - 2.0;
                0.5 * (t * t * t + 2.0)
            }
        },
        "easeInQuart" => |t| t * t * t * t,
        "easeInOutQuart" => |t| {
            let t = t / 0.5;
            if t < 1.0 {
                0.5 * t * t * t * t
            } else {
                let t = t - 2.0;
                -0.5 * (t * t * t * t - 2.0)
            }
        },
        "easeInQuint" => |t| t * t * t * t * t,
        "easeOutQuint" => |t| {
            let t = t - 1.0;
            t * t * t * t * t + 1.0
        },
        "easeInOutQuint" => |t| {
            let t = t / 0.5;
            if t < 1.0 {
                0.5 * t * t * t * t * t
            } else {
                let t = t - 2.0;
                0.5 * (t * t * t * t * t + 2.0)
            }
        },
        "easeInSine" => |t| -(t * HALF_PI).cos() + 1.0,
        "easeOutSine" => |t| (t * HALF_PI).sin(),
        "easeInOutSine" => |t| -0.5 * ((PI * t).cos() - 1.0),
        "easeInExpo" => |t| if t == 0.0 { 0.0 } else { 2f64.powf(10.0 * (t - 1.0)) },
        "easeOutExpo" => |t| if t == 1.0 { 1.0 } else { -(2f64.powf(-10.0 * t)) + 1.0 },
        "easeInOutExpo" => |t| {
            if at_edge(t) {
                t
            } else if t < 0.5 {
                0.5 * 2f64.powf(10.0 * (t * 2.0 - 1.0))
            } else {
                0.5 * (-(2f64.powf(-10.0 * (t * 2.0 - 1.0))) + 2.0)
            }
        },
        "easeInCirc" => |t| if t >= 1.0 { t } else { -((1.0 - t * t).sqrt() - 1.0) },
        "easeOutCirc" => |t| {
            let t = t - 1.0;
            (1.0 - t * t).sqrt()
        },
        "easeInOutCirc" => |t| {
            let t = t / 0.5;
            if t < 1.0 {
                -0.5 * ((1.0 - t * t).sqrt() - 1.0)
            } else {
                let t = t - 2.0;
                0.5 * ((1.0 - t * t).sqrt() + 1.0)
            }
        },
        "easeInElastic" => |t| if at_edge(t) { t } else { elastic_in(t, 0.075, 0.3) },
        "easeOutElastic" => |t| if at_edge(t) { t } else { elastic_out(t, 0.075, 0.3) },
        "easeInOutElastic" => |t| {
            let (s, p) = (0.1125, 0.45);
            if at_edge(t) {
                t
            } else if t < 0.5 {
                0.5 * elastic_in(t * 2.0, s, p)
            } else {
                0.5 + 0.5 * elastic_out(t * 2.0 - 1.0, s, p)
            }
        },
        "easeInBack" => |t| {
            let s = 1.70158;
            t * t * ((s + 1.0) * t - s)
        },
        "easeOutBack" => |t| {
            let s = 1.70158;
            let t = t - 1.0;
            t * t * ((s + 1.0) * t + s) + 1.0
        },
        "easeInOutBack" => |t| {
            let s = 1.70158 * 1.525;
            let t = t / 0.5;
            if t < 1.0 {
                0.5 * (t * t * ((s + 1.0) * t - s))
            } else {
                let t = t - 2.0;
                0.5 * (t * t * ((s + 1.0) * t + s) + 2.0)
            }
        },
        "easeInBounce" => |t| 1.0 - out_bounce(1.0 - t),
        "easeOutBounce" => out_bounce,
        "easeInOutBounce" => |t| {
            if t < 0.5 {
                (1.0 - out_bounce(1.0 - t * 2.0)) * 0.5
            } else {
                out_bounce(t * 2.0 - 1.0) * 0.5 + 0.5
            }
        },
        // easeOutQuart, Chart.js's default (and the fallback for unknown names is linear).
        "easeOutQuart" => |t| {
            let t = t - 1.0;
            -(t * t * t * t - 1.0)
        },
        _ => |t| t,
    }
}

fn lerp(from: f64, to: f64, f: f64) -> f64 {
    if from.is_finite() && to.is_finite() { from + (to - from) * f } else { to }
}

/// `interpolators.color`: `color(to).mix(color(from), factor)`, exactly as @kurkle/color
/// mixes (8-bit channels, alpha-weighted).
pub fn mix_color(from: Color, to: Color, factor: f64) -> Color {
    let b = |v: f32| (f64::from(v) * 255.0 + 0.5).floor();
    let (c1, c2) = ([b(to.r), b(to.g), b(to.b)], [b(from.r), b(from.g), b(from.b)]);
    // The `rgb` getter hands mix() alpha as 0..1 rounded to 2 decimals (`b2n`).
    let b2n = |a: f32| (f64::from(a) * 100.0 + 0.5).floor() / 100.0;
    let (a1, a2) = (b2n(to.a), b2n(from.a));
    let p = factor;
    let w = 2.0 * p - 1.0;
    let a = a1 - a2;
    let w1 = ((if w * a == -1.0 { w } else { (w + a) / (1.0 + w * a) }) + 1.0) / 2.0;
    let w2 = 1.0 - w1;
    let ch = |i: usize| ((w1 * c1[i] + w2 * c2[i] + 0.5) as i64 & 0xFF) as u8;
    let alpha = ((p * a1 + (1.0 - p) * a2) * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8;
    Color::rgba8(ch(0), ch(1), ch(2), alpha)
}

fn mix_paint(from: &Option<Paint>, to: &Option<Paint>, f: f64) -> Option<Paint> {
    match (from, to) {
        (Some(Paint::Solid(a)), Some(Paint::Solid(b))) => Some(Paint::Solid(mix_color(*a, *b, f))),
        (None, Some(Paint::Solid(b))) => Some(Paint::Solid(mix_color(Color::TRANSPARENT, *b, f))),
        _ => to.clone(),
    }
}

/// A point between `from` and `to` (properties Chart.js animates: position, radius,
/// border width and colors).
pub fn point(from: &Point, to: &Point, f: f64) -> Point {
    let mut p = to.clone();
    p.x = lerp(from.x, to.x, f);
    p.y = lerp(from.y, to.y, f);
    (p.cp1x, p.cp1y, p.cp2x, p.cp2y) = (p.x, p.y, p.x, p.y);
    p.options.radius = lerp(from.options.radius, to.options.radius, f);
    p.options.border_width = lerp(from.options.border_width, to.options.border_width, f);
    p.options.background_color = mix_paint(&from.options.background_color, &to.options.background_color, f);
    p.options.border_color = mix_paint(&from.options.border_color, &to.options.border_color, f);
    p
}

/// A bar between `from` and `to` (bars animate `x`, `y`, `base`, `width`, `height` and colors).
pub fn bar(from: &Bar, to: &Bar, f: f64) -> Bar {
    let mut b = to.clone();
    b.x = lerp(from.x, to.x, f);
    b.y = lerp(from.y, to.y, f);
    b.base = lerp(from.base, to.base, f);
    b.width = lerp(from.width, to.width, f);
    b.height = lerp(from.height, to.height, f);
    b.options.background_color = mix_paint(&from.options.background_color, &to.options.background_color, f);
    b.options.border_color = mix_paint(&from.options.border_color, &to.options.border_color, f);
    b
}

/// An arc between `from` and `to` (angles, radii, center, offset, border width, spacing,
/// colors).
pub fn arc(from: &Arc, to: &Arc, f: f64) -> Arc {
    let mut a = to.clone();
    a.x = lerp(from.x, to.x, f);
    a.y = lerp(from.y, to.y, f);
    a.start_angle = lerp(from.start_angle, to.start_angle, f);
    a.end_angle = lerp(from.end_angle, to.end_angle, f);
    a.circumference = lerp(from.circumference, to.circumference, f);
    a.inner_radius = lerp(from.inner_radius, to.inner_radius, f);
    a.outer_radius = lerp(from.outer_radius, to.outer_radius, f);
    a.options.offset = lerp(from.options.offset, to.options.offset, f);
    a.options.border_width = lerp(from.options.border_width, to.options.border_width, f);
    a.options.spacing = lerp(from.options.spacing, to.options.spacing, f);
    a.options.background_color = mix_paint(&from.options.background_color, &to.options.background_color, f);
    a.options.border_color = mix_paint(&from.options.border_color, &to.options.border_color, f);
    a
}

/// The elements one dataset showed.
#[derive(Debug, Clone, Default)]
pub struct MetaSnapshot {
    pub dataset: usize,
    pub points: Vec<Point>,
    pub bars: Vec<Bar>,
    pub arcs: Vec<Arc>,
}

/// What a chart showed: the elements of each visible dataset.
pub type Snapshot = Vec<MetaSnapshot>;

/// Which kind of update a transition animates (Chart.js's update modes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    /// The first draw: elements grow from their reset geometry.
    Initial,
    /// New data, or visibility changes: `options.animation`.
    Update,
    /// Hover changes: `transitions.active` (400 ms by default).
    Active,
}

/// A running transition.
#[derive(Debug, Clone)]
pub struct Transition {
    pub start: f64,
    pub duration: f64,
    pub delay: f64,
    pub easing: fn(f64) -> f64,
    pub looped: bool,
    pub from: Snapshot,
}

impl Transition {
    /// The eased progress at `now` (`Animation.tick`), or `None` once finished.
    pub fn progress(&self, now: f64) -> Option<f64> {
        let elapsed = now - self.start - self.delay;
        if elapsed < 0.0 {
            return Some(0.0);
        }
        if !self.looped && elapsed >= self.duration {
            return None;
        }
        let mut factor = (elapsed / self.duration) % 2.0;
        if self.looped && factor > 1.0 {
            factor = 2.0 - factor;
        }
        Some((self.easing)(factor.clamp(0.0, 1.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easings_hit_their_ends() {
        for name in [
            "linear",
            "easeInQuad",
            "easeOutQuad",
            "easeInOutQuad",
            "easeInCubic",
            "easeOutCubic",
            "easeInOutCubic",
            "easeInQuart",
            "easeOutQuart",
            "easeInOutQuart",
            "easeInQuint",
            "easeOutQuint",
            "easeInOutQuint",
            "easeInSine",
            "easeOutSine",
            "easeInOutSine",
            "easeInExpo",
            "easeOutExpo",
            "easeInOutExpo",
            "easeInCirc",
            "easeOutCirc",
            "easeInOutCirc",
            "easeInElastic",
            "easeOutElastic",
            "easeInOutElastic",
            "easeInBack",
            "easeOutBack",
            "easeInOutBack",
            "easeInBounce",
            "easeOutBounce",
            "easeInOutBounce",
        ] {
            let f = easing(name);
            assert!(f(0.0).abs() < 1e-9 && (f(1.0) - 1.0).abs() < 1e-9, "{name}: {} {}", f(0.0), f(1.0));
        }
        assert!((easing("easeOutQuart")(0.5) - 0.9375).abs() < 1e-12);
        assert!((easing("easeInOutQuad")(0.75) - 0.875).abs() < 1e-12);
        // From Chart.js helpers.easingEffects.
        assert!((easing("easeOutBounce")(0.3) - 0.680_625).abs() < 1e-12);
        assert!((easing("easeInOutBack")(0.2) - -0.092_555_656).abs() < 1e-9);
        assert!((easing("easeOutElastic")(0.4) - 1.03125).abs() < 1e-12);
    }

    #[test]
    fn color_mix_matches_kurkle() {
        // color('#ff0000').mix(color('#0000ff'), 0.25) = '#4000BF' in @kurkle/color 0.4.
        let c = mix_color(Color::rgba8(0, 0, 255, 255), Color::rgba8(255, 0, 0, 255), 0.25);
        let hex = |v: f32| (v * 255.0).round() as u8;
        assert_eq!((hex(c.r), hex(c.g), hex(c.b), hex(c.a)), (0x40, 0, 0xbf, 255));
        // color('rgba(255,0,0,0.2)').mix(color('rgba(0,0,255,0.9)'), 0.3) = '#1200EDB0'.
        let c = mix_color(Color::rgba(0.0, 0.0, 1.0, 0.9), Color::rgba(1.0, 0.0, 0.0, 0.2), 0.3);
        assert_eq!((hex(c.r), hex(c.g), hex(c.b), hex(c.a)), (0x12, 0, 0xed, 0xb0));
    }
}
