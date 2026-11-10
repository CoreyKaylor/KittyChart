//! Terminal-native chart themes.
//!
//! Chart.js's defaults assume a white page (`#666` text, `rgba(0,0,0,0.1)` grid). In a terminal
//! the chart sits on the user's theme, so KittyChart derives chart chrome and series colors from
//! the terminal's own colors. It uses the same checks a design system would use:
//!
//! - **Ink**: text and grid are the terminal foreground mixed toward the background, so they
//!   always read on that background.
//! - **Series colors** come from the terminal's ANSI palette. Each hue family's normal and
//!   bright steps are snapped into the mode's lightness band (holding hue), and the passing one
//!   that needed the smallest change is used (chroma floor, 3:1 contrast). Families keep the
//!   canonical Chart.js-like order (blue, red, yellow, green, magenta, cyan) unless that puts
//!   indistinguishable neighbors together, judged by normal-vision and simulated protan/deutan
//!   OKLab ΔE (Machado 2009). If the palette can't pass (monochrome or very low-chroma themes), a
//!   validated reference palette is used instead. The palette also needs a usable blue, the lead
//!   hue: palettes without one (muted themes) looked muddy in review, so they fall back too.
//!
//! User-specified colors always win. A theme only replaces Chart.js *defaults*.

use serde_json::{Value, json};

/// sRGB, 0–255.
pub type Rgb = [u8; 3];

/// What the terminal told us about its colors (OSC 10/11/4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalColors {
    pub background: Rgb,
    pub foreground: Rgb,
    /// ANSI 0–15, when the terminal reports them.
    pub palette: Option<[Rgb; 16]>,
}

/// Chart chrome and categorical colors for one surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    pub dark: bool,
    pub surface: Rgb,
    /// Titles.
    pub ink_primary: Rgb,
    /// Ticks, legend labels, axis titles.
    pub ink_secondary: Rgb,
    pub grid: Rgb,
    pub axis: Rgb,
    /// Series colors in assignment order.
    pub series: Vec<Rgb>,
    /// Alpha for derived fills (bars, legend boxes, areas).
    pub fill_alpha: f64,
    /// Where the series colors came from, for diagnostics.
    pub source: PaletteSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteSource {
    Terminal,
    Reference,
}

/// The dataviz reference palette (validated light/dark steps).
const REFERENCE_LIGHT: [&str; 8] =
    ["#2a78d6", "#eb6834", "#1baf7a", "#eda100", "#e87ba4", "#008300", "#6250d6", "#e34948"];
const REFERENCE_DARK: [&str; 8] =
    ["#3987e5", "#d95926", "#199e70", "#c98500", "#d55181", "#008300", "#9085e9", "#e66767"];

/// Alpha of area fills behind lines ([`Theme::area_color`]), the same as Chart.js's default
/// background colors.
pub const AREA_ALPHA: f64 = 0.5;

/// ANSI hue families: (normal, bright) palette indices. Blue first, as Chart.js leads with blue.
const FAMILIES: [(usize, usize); 6] = [(4, 12), (1, 9), (3, 11), (2, 10), (5, 13), (6, 14)];

impl Theme {
    pub fn from_terminal(t: &TerminalColors) -> Theme {
        let bg = t.background;
        let dark = luminance(bg) < 0.18;
        let fg = t.foreground;
        let (series, source) = match t.palette.as_ref().and_then(|p| terminal_series(p, bg, dark)) {
            Some(s) => (extend_series(s, bg, dark), PaletteSource::Terminal),
            None => (reference(dark), PaletteSource::Reference),
        };
        Theme {
            dark,
            surface: bg,
            ink_primary: fg,
            ink_secondary: ensure_contrast(mix(fg, bg, 0.78), fg, bg, 4.5),
            grid: mix(fg, bg, if dark { 0.13 } else { 0.11 }),
            axis: mix(fg, bg, if dark { 0.28 } else { 0.25 }),
            series,
            fill_alpha: 0.0, // set below, from the composited fills

            source,
        }
        .with_fill_alpha()
    }

    /// Fills are translucent (Chart.js style), so validate what's actually seen: each series
    /// color composited over the surface. Use the lowest alpha at which every fill keeps 3:1
    /// contrast, starting from a mode-appropriate default.
    fn with_fill_alpha(mut self) -> Theme {
        let mut alpha: f64 = if self.dark { 0.8 } else { 0.6 };
        while alpha < 1.0
            && self.series.iter().any(|c| contrast(mix(*c, self.surface, alpha), self.surface) < 3.0)
        {
            alpha += 0.05;
        }
        self.fill_alpha = (alpha.min(1.0) * 100.0).round() / 100.0;
        self
    }

    /// The theme as a Chart.js options layer, placed between user options and Chart.js defaults.
    pub fn defaults_layer(&self) -> Value {
        json!({
            "color": css(self.ink_secondary),
            "borderColor": css(self.grid),
            "backgroundColor": css(self.grid),
            // Arc separators in the surface color read as gaps on any background (Chart.js's
            // default white only works on white pages).
            "elements": { "arc": { "borderColor": css(self.surface) } },
            "plugins": {
                "title": { "color": css(self.ink_primary) },
                "subtitle": { "color": css(self.ink_secondary) }
            }
        })
    }

    /// Scale-level defaults (merged under user scale options).
    pub fn scale_layer(&self) -> Value {
        json!({
            "grid": { "color": css(self.grid) },
            "border": { "color": css(self.axis) },
            "ticks": {
                "color": css(self.ink_secondary),
                // Radial tick labels sit on a backdrop; Chart.js's is translucent white.
                "backdropColor": format!("rgba({}, {}, {}, 0.75)", self.surface[0], self.surface[1], self.surface[2])
            },
            "angleLines": { "color": css(self.grid) },
            "pointLabels": { "color": css(self.ink_secondary) },
            "title": { "color": css(self.ink_secondary) }
        })
    }

    pub fn border_color(&self, i: usize) -> String {
        css(self.series[i % self.series.len()])
    }

    /// A fill that is itself the data mark (bars, bubbles, polar segments): opaque enough to
    /// keep 3:1 contrast on the surface.
    pub fn fill_color(&self, i: usize) -> String {
        let [r, g, b] = self.series[i % self.series.len()];
        format!("rgba({r}, {g}, {b}, {})", self.fill_alpha)
    }

    /// A fill behind a line (line and radar areas, line points): the line carries the data, and
    /// the area must let what it covers show through, overlapping areas and the scale's tick
    /// labels. Chart.js's own translucency (its default backgrounds are 50% alpha).
    pub fn area_color(&self, i: usize) -> String {
        let [r, g, b] = self.series[i % self.series.len()];
        format!("rgba({r}, {g}, {b}, {AREA_ALPHA})")
    }
}

fn css([r, g, b]: Rgb) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn reference(dark: bool) -> Vec<Rgb> {
    let src = if dark { REFERENCE_DARK } else { REFERENCE_LIGHT };
    src.iter().map(|h| parse_hex(h).expect("valid reference hex")).collect()
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    Some([p(0)?, p(2)?, p(4)?])
}

/// Terminal palettes give 4–6 usable hues. Categorical colors must never cycle, so extend a
/// short palette with reference colors that are clearly distinct from every color already in it
/// (normal-vision ΔE ≥ 15) and keep 3:1 contrast on this surface.
fn extend_series(mut series: Vec<Rgb>, bg: Rgb, dark: bool) -> Vec<Rgb> {
    let (lo, hi) = if dark { (0.48, 0.67) } else { (0.43, 0.77) };
    for c in reference(dark) {
        if series.len() >= 8 {
            break;
        }
        // Snap-to-passing: hold hue, step lightness toward contrast until it reaches 3:1.
        let Some(c) = snap_for_contrast(c, bg, dark, lo, hi) else { continue };
        if series.iter().all(|s| delta_e(*s, c, None) >= 15.0) {
            series.push(c);
        }
    }
    series
}

/// Move `c`'s OKLCH lightness (hue held) away from the surface until it has 3:1 contrast,
/// staying inside the band. `None` if the band runs out first.
fn snap_for_contrast(c: Rgb, bg: Rgb, dark: bool, lo: f64, hi: f64) -> Option<Rgb> {
    let mut cur = c;
    for _ in 0..40 {
        if contrast(cur, bg) >= 3.0 {
            return Some(cur);
        }
        let l = oklch(cur).0;
        let next = if dark { l + 0.01 } else { l - 0.01 };
        if !(lo..=hi).contains(&next) {
            return None;
        }
        cur = set_lightness(cur, next);
    }
    None
}

/// Snap each ANSI hue family into the mode's lightness band (holding its hue: the
/// snap-to-passing step), keep those that pass, then pick the family order with the best
/// worst-neighbor CVD separation. `None` if fewer than four families survive or no order
/// clears the floors.
fn terminal_series(p: &[Rgb; 16], bg: Rgb, dark: bool) -> Option<Vec<Rgb>> {
    let (lo, hi) = if dark { (0.48, 0.67) } else { (0.43, 0.77) };
    let mut picks: Vec<Rgb> = Vec::new();
    for (n, b) in FAMILIES {
        // Of the normal and bright steps, take the passing one that needed the smallest
        // lightness change (closest to how the theme author intended it).
        let best = [p[n], p[b]]
            .into_iter()
            .map(|orig| (orig, snap_lightness(orig, lo, hi)))
            .filter(|(_, s)| oklch(*s).1 >= 0.10 && contrast(*s, bg) >= 3.0)
            .min_by(|(o1, s1), (o2, s2)| {
                (oklch(*o1).0 - oklch(*s1).0).abs().total_cmp(&(oklch(*o2).0 - oklch(*s2).0).abs())
            })
            .map(|(_, s)| s);
        if best.is_none() && n == FAMILIES[0].0 {
            // No usable blue: the lead hue is what makes a chart palette feel coherent (and
            // familiar from Chart.js). Without it, use the validated reference palette.
            return None;
        }
        if let Some(best) = best {
            // Drop exact-ish duplicates (sparse palettes reuse colors). Similar hues are kept:
            // keeping them apart is the ordering's job.
            if picks.iter().all(|q| delta_e(*q, best, None) >= 2.0) {
                picks.push(best);
            }
        }
    }
    if picks.len() < 4 {
        return None;
    }
    // Order. `picks` is in canonical hue order (blue, red, yellow, green, magenta, cyan: the
    // Chart.js-like lead), and the fixed order is the design rule. Among orders that clear the
    // floors (adjacent normal-vision ΔE ≥ 15, CVD ΔE ≥ 6), prefer ones that reach the CVD target
    // (≥ 8), then the fewest departures from canonical order, then the best worst-neighbor CVD.
    let n = picks.len();
    let mut best: Option<((bool, i64, f64), Vec<usize>)> = None;
    let mut idx: Vec<usize> = (0..n).collect();
    permute(&mut idx, 0, &mut |order| {
        let mut worst_cvd = f64::INFINITY;
        for w in order.windows(2) {
            let (a, b) = (picks[w[0]], picks[w[1]]);
            if delta_e(a, b, None) < 15.0 {
                return;
            }
            worst_cvd = worst_cvd.min(delta_e(a, b, Some(Cvd::Protan)).min(delta_e(a, b, Some(Cvd::Deutan))));
        }
        if worst_cvd < 6.0 {
            return;
        }
        let inversions = (0..n)
            .flat_map(|i| (i + 1..n).map(move |j| (i, j)))
            .filter(|&(i, j)| order[i] > order[j])
            .count();
        let key = (worst_cvd >= 8.0, -(inversions as i64), worst_cvd);
        let better = match &best {
            None => true,
            Some((k, _)) => (key.0, key.1).cmp(&(k.0, k.1)).then(key.2.total_cmp(&k.2)).is_gt(),
        };
        if better {
            best = Some((key, order.to_vec()));
        }
    });
    let (_, order) = best?;
    let ordered: Vec<Rgb> = order.iter().map(|&i| picks[i]).collect();
    let worst = ordered
        .windows(2)
        .map(|w| delta_e(w[0], w[1], Some(Cvd::Protan)).min(delta_e(w[0], w[1], Some(Cvd::Deutan))))
        .fold(f64::INFINITY, f64::min);
    (worst >= 6.0).then_some(ordered)
}

fn permute(v: &mut Vec<usize>, k: usize, f: &mut dyn FnMut(&[usize])) {
    if k == v.len() {
        f(v);
        return;
    }
    for i in k..v.len() {
        v.swap(k, i);
        permute(v, k + 1, f);
        v.swap(k, i);
    }
}

/// Hold hue and chroma, clamp OKLCH lightness into `[lo, hi]`, and reduce chroma until the
/// result is inside sRGB.
fn snap_lightness(c: Rgb, lo: f64, hi: f64) -> Rgb {
    let l = oklab_lin(lin(c))[0];
    // Aim slightly inside the band so 8-bit rounding can't push it out.
    set_lightness(c, l.clamp(lo + 0.005, hi - 0.005))
}

/// Hold hue, set OKLCH lightness to `target`, and reduce chroma until it fits in sRGB.
fn set_lightness(c: Rgb, target: f64) -> Rgb {
    let [_, a, b] = oklab_lin(lin(c));
    let (h, mut ch) = (b.atan2(a), a.hypot(b));
    loop {
        let rgb = oklab_to_lin([target, ch * h.cos(), ch * h.sin()]);
        if rgb.iter().all(|v| (-1e-4..=1.0001).contains(v)) || ch < 1e-3 {
            return rgb.map(|v| (lin2s(v) * 255.0).round() as u8);
        }
        ch *= 0.97;
    }
}

fn oklab_to_lin([l, a, b]: [f64; 3]) -> [f64; 3] {
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
    [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_,
    ]
}

// ---------------------------------------------------------------------------- color science
// Ports of the dataviz validator: sRGB ↔ linear, WCAG contrast, OKLab/OKLCH, and the
// Machado-Oliveira-Fernandes (2009) severity-1.0 CVD simulation.

fn s2lin(c: f64) -> f64 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn lin(c: Rgb) -> [f64; 3] {
    c.map(|v| s2lin(f64::from(v) / 255.0))
}

fn lin2s(c: f64) -> f64 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

pub fn luminance(c: Rgb) -> f64 {
    let [r, g, b] = lin(c);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

fn oklab_lin([r, g, b]: [f64; 3]) -> [f64; 3] {
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    [
        0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
        1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
        0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    ]
}

pub fn oklch(c: Rgb) -> (f64, f64) {
    let [l, a, b] = oklab_lin(lin(c));
    (l, a.hypot(b))
}

#[derive(Debug, Clone, Copy)]
enum Cvd {
    Protan,
    Deutan,
}

fn simulate(c: Rgb, kind: Cvd) -> [f64; 3] {
    let m = match kind {
        Cvd::Protan => [
            [0.152286, 1.052583, -0.204868],
            [0.114503, 0.786281, 0.099216],
            [-0.003882, -0.048116, 1.051998],
        ],
        Cvd::Deutan => {
            [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]]
        }
    };
    let [r, g, b] = lin(c);
    m.map(|row| (row[0] * r + row[1] * g + row[2] * b).clamp(0.0, 1.0))
}

/// OKLab ΔE ×100, optionally under simulated color-vision deficiency.
fn delta_e(a: Rgb, b: Rgb, kind: Option<Cvd>) -> f64 {
    let (la, lb) = match kind {
        Some(k) => (simulate(a, k), simulate(b, k)),
        None => (lin(a), lin(b)),
    };
    let (x, y) = (oklab_lin(la), oklab_lin(lb));
    100.0 * ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
}

/// Mix in gamma-encoded sRGB (close to perceptual for these purposes): `t` of `a`,
/// `1 - t` of `b`. Linear-light mixing would skew hairlines far too bright on dark surfaces.
pub fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let mut out = [0u8; 3];
    for i in 0..3 {
        out[i] = (f64::from(a[i]) * t + f64::from(b[i]) * (1.0 - t)).round() as u8;
    }
    out
}

/// Move `c` toward `toward` until it reaches `min` contrast against `bg`.
fn ensure_contrast(c: Rgb, toward: Rgb, bg: Rgb, min: f64) -> Rgb {
    let mut t = 0.0;
    let mut cur = c;
    while contrast(cur, bg) < min && t < 1.0 {
        t += 0.05;
        cur = mix(toward, c, t);
    }
    cur
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Rgb {
        parse_hex(s).unwrap()
    }

    #[test]
    fn validator_parity() {
        // Ground truth from the dataviz validator (validate_palette.js) on "#2a78d6,#eb6834":
        // CVD worst protan ΔE 24.7, normal-vision ΔE 33.6; Tokyo Night blue #7aa2f7 has L 0.719.
        let (a, b) = (h("#2a78d6"), h("#eb6834"));
        assert!((delta_e(a, b, Some(Cvd::Protan)) - 24.7).abs() < 0.05);
        assert!((delta_e(a, b, None) - 33.6).abs() < 0.05);
        assert!((oklch(h("#7aa2f7")).0 - 0.719).abs() < 0.0005);
        assert!((contrast(h("#ffffff"), h("#000000")) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn snapping_holds_hue_and_lands_in_band() {
        let c = snap_lightness(h("#b9f27c"), 0.48, 0.67); // pastel green, L 0.897
        let (l, _) = oklch(c);
        assert!((0.47..=0.675).contains(&l), "{l}");
        let hue = |c: Rgb| {
            let [_, a, b] = oklab_lin(lin(c));
            b.atan2(a).to_degrees()
        };
        assert!((hue(c) - hue(h("#b9f27c"))).abs() < 3.0);
    }

    #[test]
    fn tokyonight_uses_terminal_palette() {
        let pal = [
            "#1a1b26", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#ad8ee6", "#449dab", "#a9b1d6",
            "#414868", "#ff7a93", "#b9f27c", "#ff9e64", "#7da6ff", "#bb9af7", "#0db9d7", "#c0caf5",
        ]
        .map(h);
        let t = Theme::from_terminal(&TerminalColors {
            background: h("#1a1b26"),
            foreground: h("#a9b1d6"),
            palette: Some(pal),
        });
        assert!(t.dark);
        assert_eq!(t.source, PaletteSource::Terminal);
        assert!(t.series.len() >= 6, "short terminal palettes are extended: {:?}", t.series);
        // Neighbors stay distinguishable; appended colors are distinct from everything.
        for w in t.series.windows(2) {
            assert!(delta_e(w[0], w[1], None) >= 15.0, "adjacent colors distinguishable");
        }
        assert!(t.series.iter().all(|c| contrast(*c, t.surface) >= 3.0));
        assert!(contrast(t.ink_secondary, t.surface) >= 4.5);
        assert!(t.series.iter().all(|c| contrast(mix(*c, t.surface, t.fill_alpha), t.surface) >= 3.0));
    }

    #[test]
    fn bright_step_rescues_a_dull_normal() {
        // Normal blue and green are gray-ish; their bright variants are vivid.
        let mut pal = [
            "#000000", "#cc3333", "#6b7065", "#ccaa22", "#62656e", "#aa44aa", "#33aaaa", "#cccccc",
            "#555555", "#ff5555", "#55dd55", "#ffdd55", "#5599ff", "#dd66dd", "#55dddd", "#ffffff",
        ]
        .map(h);
        let t = Theme::from_terminal(&TerminalColors {
            background: h("#101010"),
            foreground: h("#dddddd"),
            palette: Some(pal),
        });
        assert_eq!(t.source, PaletteSource::Terminal);
        pal[12] = h("#62656e"); // bright blue dull too: no usable lead hue → reference
        let t2 = Theme::from_terminal(&TerminalColors {
            background: h("#101010"),
            foreground: h("#dddddd"),
            palette: Some(pal),
        });
        assert_eq!(t2.source, PaletteSource::Reference);
    }

    #[test]
    fn short_light_palette_is_extended_without_repeats() {
        let pal = [
            "#24292f", "#cf222e", "#116329", "#4d2d00", "#0969da", "#8250df", "#1b7c83", "#6e7781",
            "#57606a", "#a40e26", "#1a7f37", "#633c01", "#218bff", "#a475f9", "#3192aa", "#8c959f",
        ]
        .map(h);
        let t = Theme::from_terminal(&TerminalColors {
            background: h("#ffffff"),
            foreground: h("#1f2328"),
            palette: Some(pal),
        });
        assert!(t.series.len() >= 6, "{:?}", t.series);
        assert!(t.series.iter().all(|c| contrast(*c, t.surface) >= 3.0));
        let n = t.series.len();
        for i in 0..n {
            for j in i + 1..n {
                if i >= 4 || j >= 4 {
                    assert!(delta_e(t.series[i], t.series[j], None) >= 15.0, "{i} {j}");
                }
            }
        }
    }

    #[test]
    fn monochrome_falls_back_to_reference() {
        let grey = [h("#808080"); 16];
        let t = Theme::from_terminal(&TerminalColors {
            background: h("#ffffff"),
            foreground: h("#000000"),
            palette: Some(grey),
        });
        assert_eq!(t.source, PaletteSource::Reference);
        assert!(!t.dark);
    }
}
