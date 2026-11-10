//! Animation behaviour (the frames themselves are checked against Chart.js by the
//! `animation/*` parity fixtures).

use kittychart::canvas::SkiaCanvas;
use kittychart::{Chart, ChartConfig};
use kittychart_core::interaction::ActiveElement;

fn chart(options: &str) -> Chart {
    let json = format!(
        r#"{{"type":"line","data":{{"labels":["a","b","c"],"datasets":[{{"data":[1,3,2]}}]}},"options":{options}}}"#
    );
    Chart::new(ChartConfig::from_json(&json).unwrap())
}

fn draw(c: &mut Chart) {
    c.draw(&mut SkiaCanvas::new(400.0, 200.0, 1.0));
}

fn ys(c: &Chart) -> Vec<f64> {
    c.metas[0].points.iter().map(|p| p.y).collect()
}

#[test]
fn without_a_clock_charts_draw_their_final_state() {
    let mut c = chart("{}");
    draw(&mut c);
    assert!(!c.is_animating());
    let final_ys = ys(&c);
    let mut timed = chart("{}");
    timed.set_time(0.0);
    draw(&mut timed);
    assert!(timed.is_animating());
    assert_ne!(ys(&timed), final_ys, "the intro starts at the axis");
    timed.set_time(1000.0);
    draw(&mut timed);
    assert!(!timed.is_animating(), "1000 ms is the default duration");
    assert_eq!(ys(&timed), final_ys);
}

#[test]
fn animation_false_disables_it() {
    let mut c = chart(r#"{"animation": false}"#);
    c.set_time(0.0);
    draw(&mut c);
    assert!(!c.is_animating());
}

#[test]
fn hover_uses_the_active_transition() {
    let mut c = chart(r#"{"elements": {"point": {"radius": 3, "hoverRadius": 9}}}"#);
    c.set_time(0.0);
    draw(&mut c);
    c.set_time(2000.0);
    draw(&mut c);
    assert!(!c.is_animating());
    c.set_active(vec![ActiveElement { dataset: 0, index: 1 }]);
    draw(&mut c); // starts the 400 ms hover transition at t = 2000
    c.set_time(2200.0);
    draw(&mut c);
    let r = c.metas[0].points[1].options.radius;
    assert!(r > 3.0 && r < 9.0, "mid-transition radius {r}");
    assert!(c.is_animating());
    c.set_time(2400.0);
    draw(&mut c);
    assert_eq!(c.metas[0].points[1].options.radius, 9.0);
    assert!(!c.is_animating());
}

#[test]
fn updates_tween_from_the_shown_values() {
    let mut c = chart("{}");
    c.set_time(0.0);
    draw(&mut c);
    c.set_time(1000.0);
    draw(&mut c);
    let before = ys(&c);
    let mut cfg = c.config.clone();
    cfg.data["datasets"][0]["data"] = serde_json::json!([3, 1, 2]);
    c.update(cfg);
    draw(&mut c);
    assert_eq!(ys(&c), before, "the first frame of an update shows the old values");
    c.set_time(3000.0);
    draw(&mut c);
    assert_ne!(ys(&c), before);
    assert!(!c.is_animating());
}
