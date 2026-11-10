/// Non-premultiplied sRGB color with components in `0.0..=1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Color = Color::rgba(1.0, 1.0, 1.0, 1.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        let f = |v: u8| f32::from(v) / 255.0;
        Self { r: f(r), g: f(g), b: f(b), a: f(a) }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a, ..self }
    }

    pub(crate) fn to_skia(self, global_alpha: f32) -> tiny_skia::Color {
        let c = |v: f32| v.clamp(0.0, 1.0);
        tiny_skia::Color::from_rgba(c(self.r), c(self.g), c(self.b), c(self.a * global_alpha))
            .expect("clamped components are valid")
    }
}
