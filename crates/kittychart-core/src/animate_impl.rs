//! Animation driving for [`Chart`]: snapshots, reset geometry and transitions.

use crate::animation::{self, MetaSnapshot, Snapshot, Transition, TransitionKind};
use crate::chart::Chart;
use crate::config::{ChartConfig, ChartType};
use crate::scale::Axis;

/// Resolved `options.animation` for one transition.
struct TransitionConfig {
    duration: f64,
    delay: f64,
    easing: fn(f64) -> f64,
    looped: bool,
}

impl Chart {
    /// Drive animations with this clock (milliseconds; any origin). Without a clock, charts draw
    /// their final state immediately (one-shot renders). Call before each [`Chart::draw`].
    pub fn set_time(&mut self, now_ms: f64) {
        if self.anim.now.is_none() && self.anim.displayed.is_none() && self.anim.pending.is_none() {
            // The first frame with a clock plays the intro, as Chart.js does on creation.
            self.anim.pending = Some(TransitionKind::Initial);
        }
        self.anim.now = Some(now_ms);
    }

    /// Whether the last draw was mid-transition (draw again for the next frame).
    pub fn is_animating(&self) -> bool {
        self.anim.transition.is_some() || self.anim.pending.is_some()
    }

    /// Play the intro animation again.
    pub fn replay(&mut self) {
        self.anim.pending = Some(TransitionKind::Initial);
    }

    /// Replace the configuration (new data, new options) and animate to it, keeping runtime
    /// state: visibility toggles, hover, tooltip. Elements tween from where they are, like
    /// mutating `chart.data.datasets[i].data` and calling `chart.update()` in Chart.js.
    pub fn update(&mut self, config: ChartConfig) {
        self.replace_config(config);
        self.request_transition(TransitionKind::Update);
    }

    /// Start a transition on the next draw (when a clock is set). An update outranks a hover.
    pub(crate) fn request_transition(&mut self, kind: TransitionKind) {
        if self.anim.now.is_none() {
            return;
        }
        self.anim.pending = Some(match (self.anim.pending, kind) {
            (Some(TransitionKind::Initial), _) => TransitionKind::Initial,
            (Some(TransitionKind::Update), TransitionKind::Active) => TransitionKind::Update,
            (_, k) => k,
        });
    }

    /// The elements of each visible dataset.
    fn snapshot(&self) -> Snapshot {
        self.metas
            .iter()
            .filter(|m| m.visible)
            .map(|m| MetaSnapshot {
                dataset: m.index,
                points: m.points.clone(),
                bars: m.bars.clone(),
                arcs: m.arcs.clone(),
            })
            .collect()
    }

    /// The controllers' "reset" geometry for `target`: where appearing elements start.
    fn reset_snapshot(&self, target: &Snapshot) -> Snapshot {
        target
            .iter()
            .map(|t| {
                let Some(m) = self.metas.iter().position(|m| m.index == t.dataset) else { return t.clone() };
                let meta = &self.metas[m];
                let anim = self.dataset_animation_scopes(m);
                let mut s = t.clone();
                match meta.kind {
                    ChartType::Radar => {
                        if let Some(r) = &self.radial {
                            for p in &mut s.points {
                                (p.x, p.y) = (r.x_center, r.y_center);
                            }
                        }
                    }
                    ChartType::Line | ChartType::Scatter | ChartType::Bubble => {
                        let base = self.scales[meta.v_scale()].base_pixel();
                        for p in &mut s.points {
                            if meta.index_axis == Axis::X {
                                p.y = base;
                            } else {
                                p.x = base;
                            }
                            if meta.kind == ChartType::Bubble {
                                p.options.radius = 0.0;
                            }
                        }
                    }
                    ChartType::Bar => {
                        let base = self.scales[meta.v_scale()].base_pixel();
                        for b in &mut s.bars {
                            if b.horizontal {
                                (b.x, b.base, b.width) = (base, base, 0.0);
                            } else {
                                (b.y, b.base, b.height) = (base, base, 0.0);
                            }
                        }
                    }
                    ChartType::Pie | ChartType::Doughnut => {
                        let start = self.arc_start(m);
                        let (rotate, scale) = (anim.0.unwrap_or(true), anim.1.unwrap_or(false));
                        for a in &mut s.arcs {
                            if rotate {
                                (a.start_angle, a.end_angle, a.circumference) = (start, start, 0.0);
                            }
                            if scale {
                                (a.inner_radius, a.outer_radius) = (0.0, 0.0);
                            }
                        }
                    }
                    ChartType::PolarArea => {
                        let start = self.radial.as_ref().map_or(0.0, |r| r.start_angle());
                        let (rotate, scale) = (anim.0.unwrap_or(true), anim.1.unwrap_or(true));
                        for a in &mut s.arcs {
                            if rotate {
                                (a.start_angle, a.end_angle, a.circumference) = (start, start, 0.0);
                            }
                            if scale {
                                a.outer_radius = 0.0;
                            }
                        }
                    }
                }
                s
            })
            .collect()
    }

    /// The dataset's `animation.animateRotate` / `animateScale`.
    fn dataset_animation_scopes(&self, m: usize) -> (Option<bool>, Option<bool>) {
        let meta = &self.metas[m];
        let a = self.dataset_scopes_pub(meta.index, meta.kind).child("animation");
        (
            a.get("animateRotate").and_then(serde_json::Value::as_bool),
            a.get("animateScale").and_then(serde_json::Value::as_bool),
        )
    }

    /// `options.animation` for a transition kind: `(duration, delay, easing, loop)`, or `None`
    /// when animation is off.
    fn transition_config(&self, kind: TransitionKind) -> Option<TransitionConfig> {
        let s = self.chart_scopes_pub();
        if s.get("animation") == Some(&serde_json::Value::Bool(false)) {
            return None;
        }
        let base = s.child("animation");
        let mut duration = base.num_or("duration", 1000.0);
        let mut easing = base.str("easing").unwrap_or("easeOutQuart").to_owned();
        let delay = base.num_or("delay", 0.0);
        let looped = base.bool_or("loop", false);
        if kind == TransitionKind::Active {
            let t = s.child("transitions").child("active").child("animation");
            duration = t.num_or("duration", 400.0);
            if let Some(e) = t.str("easing") {
                easing = e.to_owned();
            }
        }
        (duration > 0.0).then(|| TransitionConfig {
            duration: duration.floor(),
            delay,
            easing: animation::easing(&easing),
            looped,
        })
    }

    /// Between computing the new geometry and drawing it: start pending transitions and replace
    /// the elements with their in-between state.
    pub(crate) fn animate(&mut self) {
        let Some(now) = self.anim.now else { return };
        let target = self.snapshot();
        if let Some(kind) = self.anim.pending.take() {
            self.anim.transition =
                self.transition_config(kind).map(|TransitionConfig { duration, delay, easing, looped }| {
                    let from = match (&self.anim.displayed, kind) {
                        (Some(shown), TransitionKind::Update | TransitionKind::Active) => shown.clone(),
                        _ => self.reset_snapshot(&target),
                    };
                    Transition { start: now, duration, delay, easing, looped, from }
                });
        }
        let progress = self.anim.transition.as_ref().and_then(|t| t.progress(now));
        match progress {
            None => self.anim.transition = None,
            Some(f) => {
                let tr = self.anim.transition.as_ref().expect("checked");
                let reset = self.reset_snapshot(&target);
                for meta in self.metas.iter_mut().filter(|m| m.visible) {
                    let from = tr.from.iter().find(|s| s.dataset == meta.index);
                    let reset = reset.iter().find(|s| s.dataset == meta.index);
                    for (i, p) in meta.points.iter_mut().enumerate() {
                        let a = from
                            .and_then(|s| s.points.get(i))
                            .or_else(|| reset.and_then(|s| s.points.get(i)));
                        if let Some(a) = a {
                            *p = animation::point(a, p, f);
                        }
                    }
                    for (i, b) in meta.bars.iter_mut().enumerate() {
                        let a =
                            from.and_then(|s| s.bars.get(i)).or_else(|| reset.and_then(|s| s.bars.get(i)));
                        if let Some(a) = a {
                            *b = animation::bar(a, b, f);
                        }
                    }
                    for (i, arc) in meta.arcs.iter_mut().enumerate() {
                        let a =
                            from.and_then(|s| s.arcs.get(i)).or_else(|| reset.and_then(|s| s.arcs.get(i)));
                        if let Some(a) = a {
                            *arc = animation::arc(a, arc, f);
                        }
                    }
                }
            }
        }
        self.anim.displayed = Some(self.snapshot());
    }
}
