//! A terminal-cell canvas: the text fallback for terminals without graphics.
//!
//! The whole chart engine draws onto it unchanged. Graphics (lines, bars, fills, arcs)
//! rasterize at sub-cell resolution and become block or Braille characters. Text becomes real
//! terminal characters: `measure_text` reports monospace cell widths, so Chart.js's layout
//! makes room for labels the way they will actually render.

use crate::{
    Canvas, Color, FillRule, Font, LineCap, LineJoin, Paint, SkiaCanvas, TextAlign, TextBaseline, TextMetrics,
};

/// Logical size of one terminal cell. Chart.js's 12px text with 1.2 line height is about one
/// row, and terminal cells are about twice as tall as wide.
pub const CELL_W: f32 = 8.0;
pub const CELL_H: f32 = 16.0;

/// How graphics turn into characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CellMode {
    /// Per cell, whichever reads best: quadrant blocks where the ink fills whole quadrants (bars,
    /// areas, arcs), Braille dots for finer structure (lines, points, axes).
    #[default]
    Auto,
    /// Quadrant blocks (2×2 per cell, two colors per cell). Best for bars, areas and arcs.
    Blocks,
    /// Half blocks (1×2 per cell). Most widely supported glyphs.
    HalfBlocks,
    /// Braille dots (2×4 per cell, one color per cell). Finest detail for lines.
    Braille,
}

impl CellMode {
    fn sub(self) -> (u32, u32) {
        match self {
            CellMode::Auto => (2, 4),
            CellMode::Blocks => (2, 2),
            CellMode::HalfBlocks => (1, 2),
            CellMode::Braille => (2, 4),
        }
    }
}

pub type Rgb = [u8; 3];

/// One output cell.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    pub ch: char,
    pub fg: Option<Rgb>,
    /// `None` = the terminal's own background.
    pub bg: Option<Rgb>,
    pub bold: bool,
}

/// A grid of cells, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellGrid {
    pub cols: u32,
    pub rows: u32,
    pub cells: Vec<Cell>,
}

impl CellGrid {
    pub fn get(&self, col: u32, row: u32) -> &Cell {
        &self.cells[(row * self.cols + col) as usize]
    }

    /// Just the characters, one line per row (for snapshots and plain logs).
    pub fn to_plain(&self) -> String {
        (0..self.rows)
            .map(|r| (0..self.cols).map(|c| self.get(c, r).ch).collect::<String>().trim_end().to_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Truecolor ANSI text, one line per row (separated by `\r\n`), with SGR resets per row.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();
        for r in 0..self.rows {
            if r > 0 {
                out.push_str("\r\n");
            }
            let mut cur: Option<(Option<Rgb>, Option<Rgb>, bool)> = None;
            for c in 0..self.cols {
                let cell = self.get(c, r);
                let style = (cell.fg, cell.bg, cell.bold);
                if cur != Some(style) {
                    out.push_str("\x1b[0m");
                    if cell.bold {
                        out.push_str("\x1b[1m");
                    }
                    if let Some([r, g, b]) = cell.fg {
                        out.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
                    }
                    if let Some([r, g, b]) = cell.bg {
                        out.push_str(&format!("\x1b[48;2;{r};{g};{b}m"));
                    }
                    cur = Some(style);
                }
                out.push(cell.ch);
            }
            out.push_str("\x1b[0m");
        }
        out
    }
}

#[derive(Debug, Clone)]
struct TextCell {
    ch: char,
    color: Option<Rgb>,
    bold: bool,
}

/// A [`Canvas`] whose output is terminal cells.
#[derive(Debug)]
pub struct CellCanvas {
    inner: SkiaCanvas,
    cols: u32,
    rows: u32,
    mode: CellMode,
    text: Vec<Option<TextCell>>,
    /// Auto mode: where line-like ink (strokes, small marks) is, in painter's order. Strokes
    /// paint it, region fills erase it. Cells with line ink become Braille, the rest blocks.
    lines: Option<SkiaCanvas>,
    /// The current path was filled as a region, so its outline belongs to the region.
    region_path: bool,
    /// …and it is under a cell tall (a legend box), too small for a visible outline.
    small_region: bool,
}

impl CellCanvas {
    pub fn new(cols: u32, rows: u32, mode: CellMode) -> Self {
        let (sx, sy) = mode.sub();
        // Logical units are CELL_W × CELL_H per cell; the raster has sx × sy pixels per cell.
        let inner = SkiaCanvas::with_pixel_size(cols * sx, rows * sy, sx as f32 / CELL_W);
        // Vertical scale differs from horizontal: apply it as an extra transform.
        let lines = (mode == CellMode::Auto)
            .then(|| SkiaCanvas::with_pixel_size(cols * sx, rows * sy, sx as f32 / CELL_W));
        let mut c = CellCanvas {
            inner,
            cols,
            rows,
            mode,
            text: vec![None; (cols * rows) as usize],
            lines,
            region_path: false,
            small_region: false,
        };
        c.both(|k| k.scale(1.0, (sy as f32 / CELL_H) / (sx as f32 / CELL_W)));
        c
    }

    fn both(&mut self, f: impl Fn(&mut SkiaCanvas)) {
        f(&mut self.inner);
        if let Some(l) = &mut self.lines {
            f(l);
        }
    }

    /// A filled path is a region (bar, area, slice, legend box) when it spans two cells across
    /// or one cell down, and at least a cell by half a cell; smaller fills (points) are marks
    /// drawn like lines.
    fn is_region(&self) -> bool {
        let (sx, sy) = self.mode.sub();
        self.inner.current_path_size().is_some_and(|(w, h)| {
            let (sx, sy) = (sx as f32, sy as f32);
            (w >= sx * 2.0 || h >= sy) && w >= sx && h >= sy / 2.0
        })
    }

    fn put_text(&mut self, text: &str, x: f32, y: f32) {
        let [sx, ky, kx, sy, tx, ty] = self.inner.current_transform();
        // Back to logical (cell) units: device / per-cell sub-pixel factor.
        let (psx, psy) = self.mode.sub();
        let (fx, fy) = (CELL_W / psx as f32, CELL_H / psy as f32);
        let dx = (sx * x + kx * y + tx) * fx;
        let dy = (ky * x + sy * y + ty) * fy;
        let vertical = ky.abs() > sx.abs(); // rotated about ±90°: y-axis titles
        let n = text.chars().count() as f32;
        let paint_color = match self.inner.fill_paint() {
            Paint::Solid(c) => Some(c.with_alpha(c.a * self.inner.global_alpha())),
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } | Paint::Conic { stops, .. } => {
                stops.first().map(|s| s.color)
            }
        };
        let rgb = paint_color.filter(|c| c.a > 0.05).map(to_rgb);
        let bold = self.inner.current_font().weight >= 600;
        let align = self.inner.current_text_align();
        let baseline = self.inner.current_text_baseline();
        // Offset (in cells along the text direction) from the anchor to the first character.
        let along = match align {
            TextAlign::Left | TextAlign::Start => 0.0,
            TextAlign::Right | TextAlign::End => -n,
            TextAlign::Center => -n / 2.0,
        };
        // Center line of the text across its direction, relative to the anchor, in logical px.
        let across = match baseline {
            TextBaseline::Top | TextBaseline::Hanging => CELL_H / 2.0,
            TextBaseline::Middle => 0.0,
            TextBaseline::Alphabetic => -CELL_H * 0.3,
            TextBaseline::Bottom | TextBaseline::Ideographic => -CELL_H / 2.0,
        };
        for (i, ch) in text.chars().enumerate() {
            let k = along + i as f32;
            let (col, row) = if vertical {
                // Reading direction runs up (−90°) or down (+90°) the screen.
                let up = ky < 0.0;
                let r = if up { dy / CELL_H - k - 0.5 } else { dy / CELL_H + k + 0.5 };
                let c = (dx + if up { across } else { -across }) / CELL_W;
                (c.floor(), r.floor())
            } else {
                ((dx / CELL_W + k + 0.5).floor(), ((dy + across) / CELL_H).floor())
            };
            if col < 0.0 || row < 0.0 || col >= self.cols as f32 || row >= self.rows as f32 {
                continue;
            }
            let idx = (row as u32 * self.cols + col as u32) as usize;
            if ch == ' ' {
                continue;
            }
            self.text[idx] = Some(TextCell { ch, color: rgb, bold });
        }
    }

    /// Composite graphics and text into cells.
    pub fn into_grid(self) -> CellGrid {
        let (sx, sy) = self.mode.sub();
        let px = self.inner.to_rgba();
        let lines_px = self.lines.as_ref().map(SkiaCanvas::to_rgba);
        let pw = (self.cols * sx) as usize;
        let mut cells = Vec::with_capacity((self.cols * self.rows) as usize);
        for r in 0..self.rows {
            for c in 0..self.cols {
                let idx = (r * self.cols + c) as usize;
                if let Some(t) = &self.text[idx] {
                    cells.push(Cell { ch: t.ch, fg: t.color, bg: None, bold: t.bold });
                    continue;
                }
                let mut sub: Vec<Option<Rgb>> = Vec::with_capacity((sx * sy) as usize);
                for y in 0..sy {
                    for x in 0..sx {
                        let p = ((r * sy + y) as usize * pw + (c * sx + x) as usize) * 4;
                        let a = px[p + 3];
                        sub.push((a >= 96).then(|| [px[p], px[p + 1], px[p + 2]]));
                    }
                }
                let line_ink = |x: u32, y: u32| {
                    let l = lines_px.as_ref()?;
                    let p = ((r * sy + y) as usize * pw + (c * sx + x) as usize) * 4;
                    (l[p + 3] >= 48).then_some(())
                };
                cells.push(match self.mode {
                    CellMode::Braille => braille(&sub),
                    CellMode::Auto => {
                        let dots: Vec<bool> = (0..sy)
                            .flat_map(|y| (0..sx).map(move |x| (x, y)))
                            .map(|(x, y)| line_ink(x, y).is_some())
                            .collect();
                        auto_cell(&sub, &dots)
                    }
                    CellMode::Blocks => quadrant(&sub),
                    CellMode::HalfBlocks => half_block(&sub),
                });
            }
        }
        CellGrid { cols: self.cols, rows: self.rows, cells }
    }
}

fn to_rgb(c: Color) -> Rgb {
    [(c.r * 255.0).round() as u8, (c.g * 255.0).round() as u8, (c.b * 255.0).round() as u8]
}

fn avg(colors: &[Rgb]) -> Rgb {
    let n = colors.len().max(1) as u32;
    let mut s = [0u32; 3];
    for c in colors {
        for i in 0..3 {
            s[i] += u32::from(c[i]);
        }
    }
    [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
}

fn braille(sub: &[Option<Rgb>]) -> Cell {
    // Dot bits by (x, y) for a 2×4 cell, per the Unicode Braille Patterns block.
    const BITS: [[u32; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];
    // One foreground color per cell: keep the dominant color's dots (a slice edge rather than
    // the separator next to it; the line on top where two cross).
    let near = |a: Rgb, b: Rgb| dist(a, b) < 40 * 40 * 3;
    let ink: Vec<Rgb> = sub.iter().flatten().copied().collect();
    let Some(main) = ink.iter().copied().max_by_key(|&c| ink.iter().filter(|&&o| near(c, o)).count()) else {
        return Cell { ch: ' ', ..Cell::default() };
    };
    let mut bits = 0;
    let mut inked = Vec::new();
    for y in 0..4 {
        for x in 0..2 {
            if let Some(c) = sub[y * 2 + x].filter(|&c| near(c, main)) {
                bits |= BITS[x][y];
                inked.push(c);
            }
        }
    }
    if bits == 0 {
        return Cell { ch: ' ', ..Cell::default() };
    }
    Cell { ch: char::from_u32(0x2800 + bits).unwrap_or(' '), fg: Some(avg(&inked)), bg: None, bold: false }
}

/// Hybrid: cells with line ink become Braille (dots in the line color, over the region color
/// when a region fills most of the rest of the cell); region-only cells become quadrant blocks.
fn auto_cell(sub: &[Option<Rgb>], line: &[bool]) -> Cell {
    let dots: Vec<Option<Rgb>> = sub.iter().zip(line).map(|(c, &l)| if l { *c } else { None }).collect();
    let line_ink: Vec<Rgb> = dots.iter().flatten().copied().collect();
    let region: Vec<Option<Rgb>> = sub.iter().zip(line).map(|(c, &l)| if l { None } else { *c }).collect();
    let rest: Vec<Rgb> = region.iter().flatten().copied().collect();
    if !line_ink.is_empty() {
        if rest.is_empty() {
            return braille(&dots);
        }
        // A line of the region's own color is its edge (a stacked area's border): blocks.
        if dist(avg(&line_ink), avg(&rest)) < 64 * 64 * 3 {
            return quadrants(sub);
        }
        // A distinct line across a region's interior (a dataset line over bars): its dots, on
        // the region's color. Only when the region fills (nearly) all the rest of the cell:
        // painting a half-covered cell's background would grow the region by half a cell.
        let free = dots.iter().filter(|d| d.is_none()).count();
        if rest.len() + 1 >= free {
            let mut cell = braille(&dots);
            cell.bg = Some(avg(&rest));
            return cell;
        }
        // A region's sparse corner and a few line dots: whichever has more ink.
        return if rest.len() > line_ink.len() { quadrants(&region) } else { braille(&dots) };
    }
    quadrants(sub)
}

/// Regions (and their edges, even curved ones) are quadrant blocks: solid shapes read better
/// with stair-stepped edges than with dotted fringes.
fn quadrants(sub: &[Option<Rgb>]) -> Cell {
    // Quadrant (qx, qy) is inked when either of its sub-rows is.
    let quads: Vec<Option<Rgb>> = [(0, 0), (1, 0), (0, 1), (1, 1)]
        .iter()
        .map(|&(qx, qy)| match (sub[qy * 4 + qx], sub[qy * 4 + 2 + qx]) {
            (Some(a), Some(b)) => Some(avg(&[a, b])),
            (Some(a), None) | (None, Some(a)) => Some(a),
            (None, None) => None,
        })
        .collect();
    quadrant(&quads)
}

fn half_block(sub: &[Option<Rgb>]) -> Cell {
    match (sub[0], sub[1]) {
        (None, None) => Cell { ch: ' ', ..Cell::default() },
        (Some(t), None) => Cell { ch: '▀', fg: Some(t), ..Cell::default() },
        (None, Some(b)) => Cell { ch: '▄', fg: Some(b), ..Cell::default() },
        (Some(t), Some(b)) if t == b => Cell { ch: '█', fg: Some(t), ..Cell::default() },
        (Some(t), Some(b)) => Cell { ch: '▀', fg: Some(t), bg: Some(b), bold: false },
    }
}

/// Quadrant glyphs indexed by mask (bit 0 = top-left, 1 = top-right, 2 = bottom-left, 3 = bottom-right).
const QUADRANTS: [char; 16] =
    [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];

fn dist(a: Rgb, b: Rgb) -> u32 {
    (0..3).map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2) as u32).sum()
}

fn quadrant(sub: &[Option<Rgb>]) -> Cell {
    let opaque: Vec<(usize, Rgb)> = sub.iter().enumerate().filter_map(|(i, c)| c.map(|c| (i, c))).collect();
    if opaque.is_empty() {
        return Cell { ch: ' ', ..Cell::default() };
    }
    if opaque.len() < 4 {
        // Ink over the terminal background.
        let mask = opaque.iter().fold(0, |m, (i, _)| m | (1 << i));
        let fg = avg(&opaque.iter().map(|(_, c)| *c).collect::<Vec<_>>());
        return Cell { ch: QUADRANTS[mask], fg: Some(fg), ..Cell::default() };
    }
    // All four inked: the best two-color split over the 7 distinct partitions.
    let colors: Vec<Rgb> = opaque.iter().map(|(_, c)| *c).collect();
    let mut best = (u32::MAX, 15usize, colors[0], colors[0]);
    for mask in 1..15usize {
        let fg_set: Vec<Rgb> = (0..4).filter(|i| mask & (1 << i) != 0).map(|i| colors[i]).collect();
        let bg_set: Vec<Rgb> = (0..4).filter(|i| mask & (1 << i) == 0).map(|i| colors[i]).collect();
        let (fg, bg) = (avg(&fg_set), avg(&bg_set));
        let err: u32 = fg_set.iter().map(|c| dist(*c, fg)).sum::<u32>()
            + bg_set.iter().map(|c| dist(*c, bg)).sum::<u32>();
        if err < best.0 {
            best = (err, mask, fg, bg);
        }
    }
    let all = avg(&colors);
    let solid_err: u32 = colors.iter().map(|c| dist(*c, all)).sum();
    // Near-identical halves (e.g. a translucent fill over a faint grid line) read as one color.
    if solid_err <= best.0 || dist(best.2, best.3) < 24 * 24 * 3 {
        return Cell { ch: '█', fg: Some(all), ..Cell::default() };
    }
    Cell { ch: QUADRANTS[best.1], fg: Some(best.2), bg: Some(best.3), bold: false }
}

impl Canvas for CellCanvas {
    fn width(&self) -> f32 {
        self.cols as f32 * CELL_W
    }
    fn height(&self) -> f32 {
        self.rows as f32 * CELL_H
    }
    fn save(&mut self) {
        self.both(|k| k.save());
    }
    fn restore(&mut self) {
        self.both(|k| k.restore());
    }
    fn translate(&mut self, x: f32, y: f32) {
        self.both(|k| k.translate(x, y));
    }
    fn scale(&mut self, x: f32, y: f32) {
        self.both(|k| k.scale(x, y));
    }
    fn rotate(&mut self, radians: f32) {
        self.both(|k| k.rotate(radians));
    }
    fn reset_transform(&mut self) {
        let (sx, sy) = self.mode.sub();
        self.both(|k| {
            k.reset_transform();
            k.scale(1.0, (sy as f32 / CELL_H) / (sx as f32 / CELL_W));
        });
    }
    fn set_fill_style(&mut self, paint: Paint) {
        self.inner.set_fill_style(paint);
    }
    fn set_stroke_style(&mut self, paint: Paint) {
        self.inner.set_stroke_style(paint);
    }
    fn set_line_width(&mut self, w: f32) {
        // Keep strokes at least one sub-cell wide so thin lines and grids stay visible.
        let (sx, _) = self.mode.sub();
        self.both(|k| k.set_line_width(w.max(CELL_W / sx as f32 * 0.9)));
    }
    fn set_line_cap(&mut self, cap: LineCap) {
        self.both(|k| k.set_line_cap(cap));
    }
    fn set_line_join(&mut self, join: LineJoin) {
        self.both(|k| k.set_line_join(join));
    }
    fn set_miter_limit(&mut self, limit: f32) {
        self.both(|k| k.set_miter_limit(limit));
    }
    fn set_line_dash(&mut self, segments: &[f32]) {
        self.both(|k| k.set_line_dash(segments));
    }
    fn set_line_dash_offset(&mut self, offset: f32) {
        self.both(|k| k.set_line_dash_offset(offset));
    }
    fn set_global_alpha(&mut self, alpha: f32) {
        self.inner.set_global_alpha(alpha);
    }
    fn begin_path(&mut self) {
        self.both(|k| k.begin_path());
        self.region_path = false;
    }
    fn move_to(&mut self, x: f32, y: f32) {
        self.both(|k| k.move_to(x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.both(|k| k.line_to(x, y));
    }
    fn quadratic_curve_to(&mut self, cpx: f32, cpy: f32, x: f32, y: f32) {
        self.both(|k| k.quadratic_curve_to(cpx, cpy, x, y));
    }
    fn bezier_curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.both(|k| k.bezier_curve_to(a, b, c, d, x, y));
    }
    fn arc(&mut self, cx: f32, cy: f32, r: f32, start: f32, end: f32, ccw: bool) {
        self.both(|k| k.arc(cx, cy, r, start, end, ccw));
    }
    fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, rot: f32, start: f32, end: f32, ccw: bool) {
        self.both(|k| k.ellipse(cx, cy, rx, ry, rot, start, end, ccw));
    }
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        // Snap axis-aligned rectangles (bars, legend boxes) to whole sub-cell pixels, so their
        // edges and corners don't land on partly covered sub-pixels and step unevenly.
        let [sx, ky, kx, sy, tx, ty] = self.inner.current_transform();
        let (mut x, mut y, mut w, mut h) = (x, y, w, h);
        if ky == 0.0 && kx == 0.0 && sx != 0.0 && sy != 0.0 && [x, y, w, h].iter().all(|v| v.is_finite()) {
            let snap = |v: f32, s: f32, t: f32| ((s * v + t).round() - t) / s;
            let (x0, x1) = (snap(x, sx, tx), snap(x + w, sx, tx));
            let (y0, y1) = (snap(y, sy, ty), snap(y + h, sy, ty));
            (x, y, w, h) = (x0, y0, x1 - x0, y1 - y0);
        }
        self.both(|k| k.rect(x, y, w, h));
    }
    fn close_path(&mut self) {
        self.both(|k| k.close_path());
    }
    fn fill_with_rule(&mut self, rule: FillRule) {
        let region = self.lines.is_some() && self.is_region();
        if let Some(l) = &mut self.lines {
            if region {
                l.erase_current_path();
            } else {
                l.set_fill_style(Paint::Solid(Color::WHITE));
                l.fill_with_rule(rule);
            }
        }
        if region {
            let (_, sy) = self.mode.sub();
            self.small_region = self.inner.current_path_size().is_some_and(|(_, h)| h < sy as f32);
        }
        self.region_path |= region;
        self.inner.fill_with_rule(rule);
    }
    fn stroke(&mut self) {
        if !self.region_path
            && let Some(l) = &mut self.lines
        {
            l.set_stroke_style(Paint::Solid(Color::WHITE));
            l.stroke();
        }
        if self.region_path && self.small_region {
            // An outline on a sub-cell box would swamp it: a legend box reads as one color,
            // whichever of fill and border is more colorful (the one carrying the identity: a
            // slice's fill over a surface-colored border, a line's border over a pale fill).
            let chroma = |p: &Paint| match p {
                Paint::Solid(c) => (c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b)) * c.a,
                _ => 1.0,
            };
            let fill = self.inner.fill_paint().clone();
            if chroma(self.inner.stroke_paint()) > chroma(&fill) {
                self.inner.set_fill_style(self.inner.stroke_paint().clone());
                self.inner.fill_with_rule(FillRule::NonZero);
                self.inner.set_fill_style(fill);
            }
            return;
        }
        self.inner.stroke();
    }
    fn clip(&mut self) {
        self.both(|k| k.clip());
    }
    fn clear_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.both(|k| k.clear_rect(x, y, w, h));
    }
    fn set_font(&mut self, font: Font) {
        self.inner.set_font(font);
    }
    fn set_text_align(&mut self, align: TextAlign) {
        self.inner.set_text_align(align);
    }
    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.inner.set_text_baseline(baseline);
    }
    fn fill_text(&mut self, text: &str, x: f32, y: f32, _max_width: Option<f32>) {
        self.put_text(text, x, y);
    }
    fn stroke_text(&mut self, _text: &str, _x: f32, _y: f32, _max_width: Option<f32>) {
        // Outlined text has no cell equivalent; fill_text draws the glyphs.
    }
    fn measure_text(&mut self, text: &str) -> TextMetrics {
        let w = text.chars().count() as f32 * CELL_W;
        let dy = match self.inner.current_text_baseline() {
            TextBaseline::Top | TextBaseline::Hanging => -CELL_H / 2.0,
            TextBaseline::Middle => 0.0,
            TextBaseline::Alphabetic => CELL_H * 0.3,
            TextBaseline::Bottom | TextBaseline::Ideographic => CELL_H / 2.0,
        };
        let left = match self.inner.current_text_align() {
            TextAlign::Left | TextAlign::Start => 0.0,
            TextAlign::Right | TextAlign::End => w,
            TextAlign::Center => w / 2.0,
        };
        TextMetrics {
            width: w,
            actual_bounding_box_left: left,
            actual_bounding_box_right: w - left,
            actual_bounding_box_ascent: CELL_H / 2.0 + dy,
            actual_bounding_box_descent: CELL_H / 2.0 - dy,
            font_bounding_box_ascent: CELL_H / 2.0 + dy,
            font_bounding_box_descent: CELL_H / 2.0 - dy,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Option<Rgb> = Some([255, 0, 0]);
    const B: Option<Rgb> = Some([0, 0, 255]);

    #[test]
    fn braille_bits() {
        // Left column top dot and right column bottom dot.
        let mut sub = vec![None; 8];
        sub[0] = R;
        sub[7] = R;
        assert_eq!(braille(&sub).ch, '\u{2881}');
    }

    #[test]
    fn quadrants_pick_two_colors() {
        assert_eq!(quadrant(&[R, R, R, R]).ch, '█');
        let c = quadrant(&[R, R, B, B]);
        assert_eq!((c.ch, c.fg, c.bg), ('▀', R, B));
        assert_eq!(quadrant(&[R, None, None, R]).ch, '▚');
    }

    #[test]
    fn auto_mixes_lines_and_regions() {
        let line = [false, false, true, true, false, false, false, false];
        assert_eq!(auto_cell(&[R; 8], &[false; 8]).ch, '█', "regions are blocks");

        // A distinct line across a region: its dots on the region's color.
        let mut sub = vec![B; 8];
        sub[2] = R;
        sub[3] = R;
        sub[7] = None;
        let c = auto_cell(&sub, &line);
        assert_eq!((c.ch, c.fg, c.bg), ('\u{2812}', R, B));

        // A line of the region's own color is its edge: blocks.
        let same = vec![R; 8];
        assert_eq!(auto_cell(&same, &line).ch, '█');

        // A region's corner outweighs a stray dot of another color: the corner stays.
        let mut corner = vec![None; 8];
        corner[3] = R; // a line dot (right column, row 1)
        for i in [5, 7] {
            corner[i] = B; // region: right column, rows 2-3
        }
        let mut l = [false; 8];
        l[3] = true;
        assert_eq!(auto_cell(&corner, &l).ch, '▗');

        // Mostly line, little region: Braille.
        let mut sparse = vec![None; 8];
        sparse[2] = R;
        sparse[3] = R;
        sparse[6] = B;
        let c = auto_cell(&sparse, &line);
        assert_eq!((c.ch, c.fg, c.bg), ('\u{2812}', R, None));
    }

    #[test]
    fn text_lands_on_cells() {
        let mut c = CellCanvas::new(20, 3, CellMode::Auto);
        c.set_text_align(TextAlign::Center);
        c.set_text_baseline(TextBaseline::Middle);
        c.set_fill_style(Paint::Solid(Color::WHITE));
        c.fill_text("hello", 10.5 * CELL_W, 1.5 * CELL_H, None);
        assert_eq!(c.measure_text("hello").width, 5.0 * CELL_W);
        let g = c.into_grid();
        assert_eq!(g.to_plain(), "\n        hello\n");
    }
}
