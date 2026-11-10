//! CSS color strings → canvas colors.

use kittychart_canvas::Color;

/// Parse any CSS color (`#36a2eb`, `rgb(…)`, `rgba(…)`, `hsl(…)`, named colors, `transparent`).
pub fn parse(s: &str) -> Option<Color> {
    let c = csscolorparser::parse(s.trim()).ok()?;
    Some(Color::rgba(c.r, c.g, c.b, c.a))
}

/// Chart.js's default hover color (`getHoverColor`): `color(c).saturate(0.5).darken(0.1)`,
/// computed exactly as @kurkle/color does, on 8-bit channels through HSL.
pub fn hover_color(c: Color) -> Color {
    let byte = |v: f32| ((f64::from(v) * 255.0 + 0.5).floor()).clamp(0.0, 255.0);
    let mut rgb = [byte(c.r), byte(c.g), byte(c.b)];
    rgb = mod_hsl(rgb, 1, 0.5); // saturate
    rgb = mod_hsl(rgb, 2, -0.1); // darken
    // Alpha passes through unchanged (@kurkle keeps rgba() alpha unrounded).
    Color { a: c.a, ..Color::rgba8(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8, 255) }
}

fn mod_hsl(rgb: [f64; 3], i: usize, ratio: f64) -> [f64; 3] {
    let mut hsl = rgb2hsl(rgb);
    hsl[i] = (hsl[i] + hsl[i] * ratio).clamp(0.0, if i == 0 { 360.0 } else { 1.0 });
    hsl2rgb(hsl)
}

fn rgb2hsl(v: [f64; 3]) -> [f64; 3] {
    let (r, g, b) = (v[0] / 255.0, v[1] / 255.0, v[2] / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return [0.0, 0.0, l];
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if r == max {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if g == max {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    [(h * 60.0 + 0.5).trunc(), s, l]
}

fn hsl2rgb(hsl: [f64; 3]) -> [f64; 3] {
    let (h, s, l) = (hsl[0], hsl[1], hsl[2]);
    let a = s * l.min(1.0 - l);
    let f = |n: f64| {
        let k = (n + h / 30.0) % 12.0;
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)].map(|v| (v * 255.0 + 0.5).floor().clamp(0.0, 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hover_colors_match_chartjs() {
        // Values from Chart.js 4.6.1's getHoverColor (node, helpers.getHoverColor).
        let hex = |c: Color| {
            let b = |v: f32| (v * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", b(c.r), b(c.g), b(c.b))
        };
        assert_eq!(hex(hover_color(parse("rgba(54, 162, 235, 0.5)").unwrap())), "#059bff");
        assert_eq!(hover_color(parse("rgba(54, 162, 235, 0.5)").unwrap()).a, 0.5, "alpha kept");
        assert_eq!(hex(hover_color(parse("#ff6384").unwrap())), "#ff4069");
        assert_eq!(hex(hover_color(parse("rgb(201, 203, 207)").unwrap())), "#b2b6be");
    }
}
