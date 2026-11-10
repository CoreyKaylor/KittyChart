//! Text rendering: goldens for align, baseline, rotation, weight and DPR, plus measurement
//! invariants that chart layout depends on.
#![allow(clippy::unwrap_used)]

use std::f32::consts::PI;

use kittychart_canvas::{Canvas, Color, Font, Paint, SkiaCanvas, TextAlign, TextBaseline};
use kittychart_testkit::golden::{Image, assert_golden};

fn check(name: &str, c: &SkiaCanvas) {
    let px = c.to_rgba();
    assert_golden(name, Image { width: c.pixel_width(), height: c.pixel_height(), rgba: &px });
}

fn font(size: f32, weight: u16) -> Font {
    Font { size, weight, ..Font::default() }
}

const GRAY: Color = Color::rgba(0.4, 0.4, 0.4, 1.0); // Chart.js default text color #666

#[test]
fn align_and_baseline_grid() {
    let mut c = SkiaCanvas::new(360.0, 150.0, 2.0);
    c.fill_background(Color::WHITE);
    // Guides: a vertical line at x=180 and horizontal lines at each row's y.
    c.set_stroke_style(Color::rgba8(255, 99, 132, 255).into());
    c.set_line_width(0.5);
    c.begin_path();
    c.move_to(180.0, 0.0);
    c.line_to(180.0, 150.0);
    for y in [25.0, 75.0, 125.0] {
        c.move_to(0.0, y);
        c.line_to(360.0, y);
    }
    c.stroke();

    c.set_fill_style(GRAY.into());
    c.set_font(font(14.0, 400));
    let rows = [
        (25.0, TextAlign::Left, TextBaseline::Top),
        (75.0, TextAlign::Center, TextBaseline::Middle),
        (125.0, TextAlign::Right, TextBaseline::Alphabetic),
    ];
    for (y, align, base) in rows {
        c.set_text_align(align);
        c.set_text_baseline(base);
        c.fill_text(&format!("{align:?}/{base:?} Ágjy 123"), 180.0, y, None);
    }
    check("canvas/text_align_baseline", &c);
}

#[test]
fn rotated_tick_labels_bold_title_and_alpha() {
    let mut c = SkiaCanvas::new(240.0, 140.0, 1.0);
    c.fill_background(Color::WHITE);
    c.set_font(font(16.0, 700));
    c.set_fill_style(GRAY.into());
    c.set_text_align(TextAlign::Center);
    c.set_text_baseline(TextBaseline::Top);
    c.fill_text("Monthly Revenue", 120.0, 8.0, None);

    // Chart.js-style rotated x-axis labels: rotate around the label anchor, align right.
    c.set_font(font(12.0, 400));
    c.set_text_align(TextAlign::Right);
    c.set_text_baseline(TextBaseline::Middle);
    for (i, label) in ["January", "February", "March", "April"].iter().enumerate() {
        c.save();
        c.translate(50.0 + i as f32 * 50.0, 50.0);
        c.rotate(-PI / 4.0);
        c.set_fill_style(Color::rgba8(54, 162, 235, 255).with_alpha(0.5 + 0.15 * i as f32).into());
        c.fill_text(label, 0.0, 0.0, None);
        c.restore();
    }
    // Gradient-filled text and a font stack that isn't installed (falls back to Inter).
    c.set_font(Font {
        family: "'Definitely Not Installed', sans-serif".into(),
        size: 20.0,
        weight: 700,
        italic: false,
    });
    c.set_text_align(TextAlign::Left);
    c.set_text_baseline(TextBaseline::Alphabetic);
    c.set_fill_style(Paint::Linear {
        x0: 130.0,
        y0: 0.0,
        x1: 230.0,
        y1: 0.0,
        stops: vec![
            kittychart_canvas::ColorStop { offset: 0.0, color: Color::rgba8(255, 99, 132, 255) },
            kittychart_canvas::ColorStop { offset: 1.0, color: Color::rgba8(153, 102, 255, 255) },
        ],
    });
    c.fill_text("42.7%", 130.0, 130.0, None);
    check("canvas/text_rotated_bold_gradient", &c);
}

#[test]
fn measure_text_invariants() {
    let mut c = SkiaCanvas::new(10.0, 10.0, 1.0);
    c.set_font(font(12.0, 400));
    let a = c.measure_text("Revenue");
    let b = c.measure_text("Revenue Revenue");
    assert!(a.width > 30.0 && a.width < 60.0, "12px 'Revenue' width {}", a.width);
    assert!(b.width > 2.0 * a.width, "space adds width");
    assert_eq!(c.measure_text("").width, 0.0);

    // Width is linear in font size (unhinted outlines, no pixel snapping).
    c.set_font(font(24.0, 400));
    let a2 = c.measure_text("Revenue");
    assert!((a2.width - 2.0 * a.width).abs() < 0.01, "{} vs {}", a2.width, a.width);

    // Bold is wider than regular.
    c.set_font(font(12.0, 700));
    assert!(c.measure_text("Revenue").width > a.width);

    // DPR does not change logical measurements.
    let mut hi = SkiaCanvas::new(10.0, 10.0, 3.0);
    hi.set_font(font(12.0, 400));
    assert_eq!(hi.measure_text("Revenue").width, a.width);

    // Baselines shift font box metrics the way the web does.
    c.set_font(font(20.0, 400));
    c.set_text_baseline(TextBaseline::Alphabetic);
    let alpha = c.measure_text("Hg");
    assert!(alpha.font_bounding_box_ascent > 15.0 && alpha.font_bounding_box_descent > 3.0);
    assert!(alpha.actual_bounding_box_ascent > 13.0, "cap height of H");
    assert!(alpha.actual_bounding_box_descent > 3.0, "descender of g");
    c.set_text_baseline(TextBaseline::Top);
    let top = c.measure_text("Hg");
    // `top` puts the normalized em box's top at y (≈0.8em above the baseline for Inter), so the
    // taller font box pokes above y.
    let em_ascent = alpha.font_bounding_box_ascent
        / (alpha.font_bounding_box_ascent + alpha.font_bounding_box_descent)
        * 20.0;
    assert!((top.font_bounding_box_ascent - (alpha.font_bounding_box_ascent - em_ascent)).abs() < 0.01);
    assert!((em_ascent - 16.0).abs() < 0.1, "Inter's em ascent ≈ 0.8em, got {em_ascent}");
    c.set_text_baseline(TextBaseline::Middle);
    let mid = c.measure_text("Hg");
    // middle = center of the em box: (emAscent - emDescent) / 2 ≈ 0.3em above the baseline.
    let em_descent = 20.0 - em_ascent;
    assert!(
        (alpha.font_bounding_box_ascent - mid.font_bounding_box_ascent - (em_ascent - em_descent) / 2.0)
            .abs()
            < 0.01
    );

    // Alignment moves the ink box.
    c.set_text_baseline(TextBaseline::Alphabetic);
    c.set_text_align(TextAlign::Right);
    let right = c.measure_text("Hg");
    assert!(right.actual_bounding_box_right <= 0.5 && right.actual_bounding_box_left > 15.0);
}

#[test]
fn max_width_condenses_and_ellipse_draws() {
    let mut c = SkiaCanvas::new(200.0, 60.0, 1.0);
    c.fill_background(Color::WHITE);
    c.set_font(font(14.0, 400));
    c.set_fill_style(GRAY.into());
    c.set_text_align(TextAlign::Center);
    c.set_text_baseline(TextBaseline::Middle);
    c.fill_text("A rather long chart title that will not fit", 100.0, 15.0, Some(180.0));
    c.begin_path();
    c.ellipse(100.0, 42.0, 30.0, 10.0, 0.0, 0.0, 2.0 * PI, false);
    c.set_fill_style(Color::rgba8(75, 192, 192, 255).into());
    c.fill();
    check("canvas/text_max_width_ellipse", &c);
}
