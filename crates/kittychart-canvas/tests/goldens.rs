//! L1 render goldens for canvas primitives. They pin down the backend's anti-aliasing, DPR
//! scaling, dashes, arcs, clipping and gradients before chart code is built on top of them.
#![allow(clippy::unwrap_used)]

use std::f32::consts::PI;

use kittychart_canvas::{Canvas, Color, ColorStop, LineCap, LineJoin, Paint, SkiaCanvas};
use kittychart_testkit::golden::{Image, assert_golden};

fn check(name: &str, c: &SkiaCanvas) {
    let px = c.to_rgba();
    assert_golden(name, Image { width: c.pixel_width(), height: c.pixel_height(), rgba: &px });
}

fn hex(rgb: u32) -> Color {
    Color::rgba8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255)
}

#[test]
fn primitives_at_dpr_2() {
    let mut c = SkiaCanvas::new(160.0, 100.0, 2.0);
    c.fill_background(Color::WHITE);
    assert_eq!((c.pixel_width(), c.pixel_height()), (320, 200));

    // Chart.js "blue" with a translucent fill, as in an area chart.
    c.set_fill_style(hex(0x36a2eb).with_alpha(0.5).into());
    c.fill_rect(10.0, 10.0, 60.0, 40.0);
    c.set_stroke_style(hex(0x36a2eb).into());
    c.set_line_width(2.0);
    c.stroke_rect(10.0, 10.0, 60.0, 40.0);

    // Dashed polyline with round joins.
    c.begin_path();
    c.move_to(80.0, 50.0);
    c.line_to(110.0, 15.0);
    c.line_to(150.0, 40.0);
    c.set_stroke_style(hex(0xff6384).into());
    c.set_line_width(3.0);
    c.set_line_join(LineJoin::Round);
    c.set_line_cap(LineCap::Round);
    c.set_line_dash(&[6.0, 4.0]);
    c.stroke();

    // Pie wedge: arc + line back to the center.
    c.set_line_dash(&[]);
    c.begin_path();
    c.move_to(40.0, 75.0);
    c.arc(40.0, 75.0, 20.0, -PI / 2.0, PI / 3.0, false);
    c.close_path();
    c.set_fill_style(hex(0xffcd56).into());
    c.fill();
    c.set_stroke_style(Color::WHITE.into());
    c.set_line_width(2.0);
    c.stroke();
    check("canvas/primitives_dpr2", &c);
}

#[test]
fn clip_transform_and_gradient() {
    let mut c = SkiaCanvas::new(120.0, 80.0, 1.0);
    c.fill_background(Color::WHITE);
    c.save();
    c.begin_path();
    c.arc(60.0, 40.0, 30.0, 0.0, 2.0 * PI, false);
    c.clip();
    c.set_fill_style(Paint::Linear {
        x0: 0.0,
        y0: 0.0,
        x1: 120.0,
        y1: 0.0,
        stops: vec![
            ColorStop { offset: 0.0, color: hex(0x4bc0c0) },
            ColorStop { offset: 1.0, color: hex(0x9966ff) },
        ],
    });
    c.fill_rect(0.0, 0.0, 120.0, 80.0);
    c.restore();

    // After restore the clip is gone; a rotated square at the edge is drawn whole.
    c.save();
    c.translate(105.0, 15.0);
    c.rotate(PI / 4.0);
    c.set_fill_style(hex(0xff9f40).into());
    c.fill_rect(-6.0, -6.0, 12.0, 12.0);
    c.restore();
    check("canvas/clip_transform_gradient", &c);
}

#[test]
fn logical_size_is_independent_of_dpr() {
    for dpr in [1.0, 1.5, 2.0, 3.0] {
        let c = SkiaCanvas::new(100.0, 50.0, dpr);
        assert!((c.width() - 100.0).abs() < 1.0 / dpr, "dpr {dpr}");
        assert!((c.height() - 50.0).abs() < 1.0 / dpr, "dpr {dpr}");
    }
}

#[test]
fn non_finite_path_arguments_are_ignored() {
    // As in browsers: a NaN arc (e.g. 0/0 angles from a pie's zero inner radius) must not
    // corrupt the rest of the path.
    let mut c = SkiaCanvas::new(20.0, 20.0, 1.0);
    c.begin_path();
    c.move_to(2.0, 2.0);
    c.line_to(18.0, 2.0);
    c.arc(10.0, 10.0, 0.0, f32::NAN, f32::NAN, true);
    c.line_to(f32::INFINITY, 5.0);
    c.line_to(18.0, 18.0);
    c.close_path();
    c.set_fill_style(Color::BLACK.into());
    c.fill();
    let px = c.to_rgba();
    let at = |x: usize, y: usize| px[(y * 20 + x) * 4 + 3];
    assert_eq!(at(16, 6), 255, "triangle interior is filled");
    assert_eq!(at(3, 16), 0, "outside stays clear");
}
