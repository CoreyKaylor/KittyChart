//! CSS color strings → canvas colors.

use kittychart_canvas::Color;

/// Parse any CSS color (`#36a2eb`, `rgb(…)`, `rgba(…)`, `hsl(…)`, named colors, `transparent`).
pub fn parse(s: &str) -> Option<Color> {
    let c = csscolorparser::parse(s.trim()).ok()?;
    Some(Color::rgba(c.r, c.g, c.b, c.a))
}
