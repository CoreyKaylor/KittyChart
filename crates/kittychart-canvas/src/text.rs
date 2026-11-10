//! Text shaping and glyph outlines.
//!
//! cosmic-text shapes (harfrust) and handles font fallback. Glyphs are drawn as **outlines**
//! filled through the same tiny-skia path pipeline as every other shape, so rotated tick
//! labels, clips, global alpha and gradient fills all behave exactly as for paths.
//! Outlines are unhinted and cached in font units, so one entry serves every size and transform.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use cosmic_text::skrifa;
use cosmic_text::{
    Attrs, Buffer, CacheKeyFlags, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap, fontdb,
};
use skrifa::MetadataProvider as _;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use tiny_skia as sk;

/// Inter, bundled so output is identical on every machine. SIL Open Font License 1.1, see
/// `fonts/Inter-OFL.txt`.
pub const INTER_REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
pub const INTER_BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");
pub const BUNDLED_FAMILY: &str = "Inter";

/// A CSS-like font: the parts of Chart.js's `FontSpec` that affect rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    /// CSS font-family list, such as `"'Helvetica Neue', Arial, sans-serif"`. The first family
    /// that is installed wins, then the generic families, then Inter.
    pub family: String,
    /// Size in logical pixels.
    pub size: f32,
    /// CSS numeric weight (400 normal, 700 bold).
    pub weight: u16,
    pub italic: bool,
}

impl Default for Font {
    /// Chart.js's `Chart.defaults.font`.
    fn default() -> Self {
        Self {
            family: "'Helvetica Neue', 'Helvetica', 'Arial', sans-serif".into(),
            size: 12.0,
            weight: 400,
            italic: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Start,
    End,
    Left,
    Right,
    Center,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextBaseline {
    #[default]
    Alphabetic,
    Top,
    Hanging,
    Middle,
    Ideographic,
    Bottom,
}

/// The subset of the web's `TextMetrics`. Bounding-box values are relative to the drawing point
/// for the current align and baseline, and positive values point away from it (as on the web).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TextMetrics {
    pub width: f32,
    pub actual_bounding_box_left: f32,
    pub actual_bounding_box_right: f32,
    pub actual_bounding_box_ascent: f32,
    pub actual_bounding_box_descent: f32,
    pub font_bounding_box_ascent: f32,
    pub font_bounding_box_descent: f32,
}

/// Fonts plus caches. Creating one is cheap for [`FontContext::bundled`]. Scanning system
/// fonts ([`FontContext::with_system_fonts`]) costs real time and is the only I/O here.
pub struct FontContext {
    fs: FontSystem,
    /// Glyph outlines in font units (y-up), keyed by (face, glyph).
    outlines: HashMap<(fontdb::ID, u16), Option<sk::Path>>,
    /// (units_per_em, ascent, descent) in font units, descent positive.
    faces: HashMap<fontdb::ID, (f32, f32, f32)>,
}

impl std::fmt::Debug for FontContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FontContext").field("faces", &self.fs.db().len()).finish_non_exhaustive()
    }
}

impl FontContext {
    /// Only the bundled Inter faces. Deterministic: used by tests and as the default.
    pub fn bundled() -> Self {
        let mut db = fontdb::Database::new();
        db.load_font_data(INTER_REGULAR.to_vec());
        db.load_font_data(INTER_BOLD.to_vec());
        Self::from_db(db)
    }

    /// Bundled fonts plus every installed system font, so `font.family` can name them.
    pub fn with_system_fonts() -> Self {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        db.load_font_data(INTER_REGULAR.to_vec());
        db.load_font_data(INTER_BOLD.to_vec());
        Self::from_db(db)
    }

    fn from_db(mut db: fontdb::Database) -> Self {
        // All generic families fall back to Inter. Chart.js's default stack ends in sans-serif.
        db.set_sans_serif_family(BUNDLED_FAMILY);
        db.set_serif_family(BUNDLED_FAMILY);
        db.set_monospace_family(BUNDLED_FAMILY);
        db.set_cursive_family(BUNDLED_FAMILY);
        db.set_fantasy_family(BUNDLED_FAMILY);
        Self {
            fs: FontSystem::new_with_locale_and_db("en-US".into(), db),
            outlines: HashMap::new(),
            faces: HashMap::new(),
        }
    }

    /// Add a font file (TTF/OTF/TTC bytes).
    pub fn load_font_data(&mut self, data: Vec<u8>) {
        self.fs.db_mut().load_font_data(data);
    }

    /// The process-wide bundled context used by canvases unless one is supplied.
    pub fn shared_bundled() -> Arc<Mutex<FontContext>> {
        static CTX: OnceLock<Arc<Mutex<FontContext>>> = OnceLock::new();
        CTX.get_or_init(|| Arc::new(Mutex::new(FontContext::bundled()))).clone()
    }

    /// Resolve a CSS family list to a single family name present in the database.
    fn resolve_family(&self, list: &str) -> String {
        for raw in list.split(',') {
            let name = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            if name.is_empty() {
                continue;
            }
            let generic = matches!(
                name.to_ascii_lowercase().as_str(),
                "sans-serif" | "serif" | "monospace" | "cursive" | "fantasy" | "system-ui"
            );
            if generic {
                return BUNDLED_FAMILY.into();
            }
            let present =
                self.fs.db().faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)));
            if present {
                return name.into();
            }
        }
        BUNDLED_FAMILY.into()
    }

    /// Shape one line of text. Newlines are treated as spaces, as in Canvas `fillText`.
    pub(crate) fn shape(&mut self, text: &str, font: &Font) -> Shaped {
        let family = self.resolve_family(&font.family);
        let attrs = Attrs::new()
            .family(Family::Name(&family))
            .weight(Weight(font.weight))
            .style(if font.italic { Style::Italic } else { Style::Normal });
        let size = font.size.max(0.0);
        let line: String = text.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();

        let mut buf = Buffer::new(&mut self.fs, Metrics::new(size.max(0.01), size.max(0.01) * 1.2));
        buf.set_wrap(Wrap::None);
        buf.set_size(None, None);
        buf.set_text(&line, &attrs, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.fs, false);

        let mut glyphs = Vec::new();
        let mut width = 0.0f32;
        for run in buf.layout_runs() {
            width = width.max(run.line_w);
            for g in run.glyphs {
                glyphs.push(ShapedGlyph {
                    face: g.font_id,
                    glyph: g.glyph_id,
                    weight: g.font_weight,
                    x: g.x + g.x_offset * g.font_size,
                    y: g.y - g.y_offset * g.font_size,
                    size: g.font_size,
                    fake_italic: g.cache_key_flags.contains(CacheKeyFlags::FAKE_ITALIC),
                });
            }
        }

        // Font-level ascent/descent come from the primary face. That is the first glyph's face,
        // or a database query for empty text.
        let primary = glyphs.first().map(|g| (g.face, g.weight)).or_else(|| {
            let q = fontdb::Query {
                families: &[fontdb::Family::Name(&family)],
                weight: fontdb::Weight(font.weight),
                stretch: fontdb::Stretch::Normal,
                style: if font.italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
            };
            self.fs.db().query(&q).map(|id| (id, fontdb::Weight(font.weight)))
        });
        let (ascent, descent) = primary
            .and_then(|(id, w)| self.face_metrics(id, w))
            .map_or((size * 0.9, size * 0.25), |(upem, a, d)| (a / upem * size, d / upem * size));

        Shaped { glyphs, width, ascent, descent, size }
    }

    fn face_metrics(&mut self, id: fontdb::ID, weight: fontdb::Weight) -> Option<(f32, f32, f32)> {
        if let Some(m) = self.faces.get(&id) {
            return Some(*m);
        }
        let font = self.fs.get_font(id, weight)?;
        let r = skrifa::FontRef::from_index(font.data(), 0).ok()?;
        let m = r.metrics(Size::unscaled(), LocationRef::default());
        let v = (f32::from(m.units_per_em), m.ascent, -m.descent);
        self.faces.insert(id, v);
        Some(v)
    }

    /// Glyph outline in font units, with the em scale. `None` for blank glyphs such as space.
    pub(crate) fn outline(&mut self, g: &ShapedGlyph) -> Option<(sk::Path, f32)> {
        let (upem, _, _) = self.face_metrics(g.face, g.weight)?;
        let key = (g.face, g.glyph);
        if !self.outlines.contains_key(&key) {
            let path = self.fs.get_font(g.face, g.weight).and_then(|font| {
                let r = skrifa::FontRef::from_index(font.data(), 0).ok()?;
                let og = r.outline_glyphs().get(skrifa::GlyphId::new(u32::from(g.glyph)))?;
                let mut pen = PathPen(sk::PathBuilder::new());
                og.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).ok()?;
                pen.0.finish()
            });
            self.outlines.insert(key, path);
        }
        self.outlines[&key].clone().map(|p| (p, upem))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ShapedGlyph {
    face: fontdb::ID,
    glyph: u16,
    weight: fontdb::Weight,
    /// Pen position relative to the line origin, on the alphabetic baseline, in logical px.
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub fake_italic: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Shaped {
    pub glyphs: Vec<ShapedGlyph>,
    pub width: f32,
    /// Font ascent and descent in logical px, both positive.
    pub ascent: f32,
    pub descent: f32,
    pub size: f32,
}

impl Shaped {
    /// Offset from the requested `x` to the line's left edge.
    pub fn align_dx(&self, align: TextAlign) -> f32 {
        match align {
            TextAlign::Start | TextAlign::Left => 0.0,
            TextAlign::End | TextAlign::Right => -self.width,
            TextAlign::Center => -self.width / 2.0,
        }
    }

    /// Em-box ascent/descent: font ascent and descent rescaled so they sum to 1em.
    /// Chromium and Skia place `top`/`middle`/`bottom` baselines on this box.
    fn em_box(&self) -> (f32, f32) {
        let total = self.ascent + self.descent;
        if total <= 0.0 {
            return (self.size * 0.8, self.size * 0.2);
        }
        (self.ascent / total * self.size, self.descent / total * self.size)
    }

    /// Offset from the requested `y` to the alphabetic baseline, matching Chromium's
    /// `TextMetrics::GetFontBaseline`: top/middle/bottom use the normalized em box,
    /// hanging is 80% of the font ascent, and ideographic is the font descent.
    pub fn baseline_dy(&self, baseline: TextBaseline) -> f32 {
        let (em_ascent, em_descent) = self.em_box();
        match baseline {
            TextBaseline::Alphabetic => 0.0,
            TextBaseline::Top => em_ascent,
            TextBaseline::Hanging => self.ascent * 0.8,
            TextBaseline::Middle => (em_ascent - em_descent) / 2.0,
            TextBaseline::Bottom => -em_descent,
            TextBaseline::Ideographic => -self.descent,
        }
    }
}

struct PathPen(sk::PathBuilder);

impl OutlinePen for PathPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(cx0, cy0, x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(cx0, cy0, cx1, cy1, x, y);
    }
    fn close(&mut self) {
        self.0.close();
    }
}
