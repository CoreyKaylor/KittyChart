//! An interactive chart in a terminal region: sans-IO. Hosts feed it input events and a
//! clock, and write the bytes it returns.
//!
//! ```text
//! let mut s = Session::new(config, info, SessionOptions::default());
//! loop {
//!     for ev in parser.feed(&read_input()) { if s.handle(&ev) == Action::Quit { break } }
//!     if let Some(bytes) = s.frame(now_ms()) { write(bytes) }
//! }
//! write(s.cleanup());
//! ```
//!
//! Hover follows Chart.js: `hover`/`interaction` options pick the active elements (drawn with
//! their hover styles), the tooltip's own mode picks what it lists, and a click on a legend
//! item toggles that dataset (or slice). With kitty graphics the image is re-sent in place
//! (same id and placement), through a temp file or shared memory when the terminal supports it.

use kittychart_canvas::SkiaCanvas;
use kittychart_canvas::cells::{CELL_H, CELL_W, CellCanvas, CellMode};
use kittychart_core::interaction::ActiveElement;
use kittychart_term::RgbaImage;
use kittychart_term::input::{Button, Event, Key, MouseKind};
use kittychart_term::kitty::{self, Medium, TransmitOptions};
use kittychart_term::sixel::{self, SixelOptions};

use crate::show::{Mode, TerminalInfo};
use crate::{Chart, ChartConfig, Color, Theme};

/// Where and how a session draws.
#[derive(Debug, Clone)]
pub struct SessionOptions {
    /// Output: [`Mode::Kitty`], [`Mode::Sixel`] or a text mode ([`Mode::Auto`] picks the best).
    pub mode: Mode,
    /// The region, in cells: top-left corner (0-based) and size. Default: the whole screen
    /// except the last row (the status line).
    pub region: Option<Region>,
    /// Device pixels per CSS pixel (default: chart text about as tall as terminal text).
    pub scale: Option<f32>,
    pub background: Option<Color>,
    /// Use Chart.js's own default colors instead of the terminal's.
    pub chartjs_colors: bool,
    /// kitty image id.
    pub image_id: u32,
    /// How kitty images travel; `Medium::Direct` by default. A temp file or shared memory is
    /// much faster for animation (see `kittychart_term::tty::Tty::probe_media`).
    pub medium: Medium,
    /// Minimum milliseconds between frames (default 33: about 30 per second).
    pub frame_interval: f64,
    /// Draw a one-line key hint below the chart.
    pub status_line: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        SessionOptions {
            mode: Mode::Auto,
            region: None,
            scale: None,
            background: None,
            chartjs_colors: false,
            image_id: crate::show::next_image_id(),
            medium: Medium::Direct,
            frame_interval: 33.0,
            status_line: true,
        }
    }
}

/// A rectangle of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub col: u16,
    pub row: u16,
    pub cols: u16,
    pub rows: u16,
}

/// What the host should do after an event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    /// Something changed; the next [`Session::frame`] will draw.
    Redraw,
    Quit,
    /// The user asked to save the chart (`s`): here it is as a PNG.
    Save(Vec<u8>),
}

/// An interactive chart.
#[derive(Debug)]
pub struct Session {
    chart: Chart,
    config: ChartConfig,
    info: TerminalInfo,
    opts: SessionOptions,
    mode: Mode,
    region: Region,
    scale: f32,
    pointer: Option<(f64, f64)>,
    /// Arrow-key position (index mode), when not following the pointer.
    keyed: Option<usize>,
    dirty: bool,
    last_frame: Option<f64>,
    sent: bool,
    /// The kitty image on screen: `opts.image_id` or its [`kitty::alternate_id`]. Each frame
    /// goes out under the other id and then the old image is deleted, because re-sending under
    /// one id blanks the image until the transfer completes (a visible flicker with large
    /// direct transfers).
    shown: Option<u32>,
    /// Clear the screen before the next frame: the terminal was resized, and what the last
    /// size drew (sixel pixels, the old status line under a transparent image) would remain.
    clear: bool,
}

impl Session {
    pub fn new(config: ChartConfig, info: TerminalInfo, opts: SessionOptions) -> Session {
        let mode = if opts.mode == Mode::Auto { info.best_mode() } else { opts.mode };
        let mut s = Session {
            chart: Chart::new(config.clone()),
            config,
            region: Region { col: 0, row: 0, cols: 1, rows: 1 },
            scale: 1.0,
            info,
            opts,
            mode,
            pointer: None,
            keyed: None,
            dirty: true,
            last_frame: None,
            sent: false,
            shown: None,
            clear: false,
        };
        s.rebuild();
        s
    }

    /// The chart (for inspection).
    pub fn chart(&self) -> &Chart {
        &self.chart
    }

    /// Output mode in use.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    fn theme(&self) -> Option<Theme> {
        (!self.opts.chartjs_colors).then(|| self.info.colors.as_ref().map(Theme::from_terminal)).flatten()
    }

    /// (Re)build the chart for the current mode and region, keeping nothing but the config.
    fn rebuild(&mut self) {
        let screen = Region {
            col: 0,
            row: 0,
            cols: self.info.cols.max(10),
            rows: self.info.rows.saturating_sub(u16::from(self.opts.status_line)).max(4),
        };
        self.region = self.opts.region.unwrap_or(screen);
        self.scale = self.opts.scale.unwrap_or_else(|| crate::terminal::default_scale(self.info.cell_height));
        let theme = self.theme();
        self.chart = if matches!(self.mode, Mode::Text(_)) {
            Chart::for_cells(self.config.clone(), theme)
        } else {
            Chart::with_theme(self.config.clone(), theme)
        };
        self.chart.dpr = f64::from(self.scale);
        self.dirty = true;
    }

    /// The terminal changed size.
    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: f32, cell_height: f32) {
        if (cols, rows, cell_width, cell_height)
            == (self.info.cols, self.info.rows, self.info.cell_width, self.info.cell_height)
        {
            return;
        }
        (self.info.cols, self.info.rows, self.info.cell_width, self.info.cell_height) =
            (cols, rows, cell_width, cell_height);
        // Runtime state stays: the chart is sized at draw time anyway.
        if self.opts.region.is_none() {
            // The session owns the whole screen.
            self.clear = true;
            self.region.cols = cols.max(10);
            self.region.rows = rows.saturating_sub(u16::from(self.opts.status_line)).max(4);
        }
        self.scale = self.opts.scale.unwrap_or_else(|| crate::terminal::default_scale(cell_height));
        self.chart.dpr = f64::from(self.scale);
        self.dirty = true;
        self.sent = false;
    }

    /// New data or options: animate to them.
    pub fn update(&mut self, config: ChartConfig) {
        self.config = config.clone();
        self.chart.update(config);
        self.dirty = true;
    }

    /// The chart area's size in device pixels.
    fn pixel_size(&self) -> (u32, u32) {
        (
            (f32::from(self.region.cols) * self.info.cell_width).round() as u32,
            (f32::from(self.region.rows) * self.info.cell_height).round() as u32,
        )
    }

    /// Terminal pointer position → chart CSS pixels. Pixel reports (mode 1016) are exact; cell
    /// reports use the cell's center.
    fn to_chart(&self, x: u32, y: u32, pixels: bool) -> Option<(f64, f64)> {
        let (cw, ch) = if matches!(self.mode, Mode::Text(_)) {
            (f64::from(CELL_W), f64::from(CELL_H))
        } else {
            (f64::from(self.info.cell_width), f64::from(self.info.cell_height))
        };
        let (px, py) = if pixels && !matches!(self.mode, Mode::Text(_)) {
            (f64::from(x), f64::from(y))
        } else if pixels {
            // Pixel report in text mode: to cells first.
            let cx = f64::from(x) / f64::from(self.info.cell_width);
            let cy = f64::from(y) / f64::from(self.info.cell_height);
            (cx * cw, cy * ch)
        } else {
            ((f64::from(x) + 0.5) * cw, (f64::from(y) + 0.5) * ch)
        };
        let (ox, oy) = (f64::from(self.region.col) * cw, f64::from(self.region.row) * ch);
        let (w, h) = (f64::from(self.region.cols) * cw, f64::from(self.region.rows) * ch);
        let (rx, ry) = (px - ox, py - oy);
        if rx < 0.0 || ry < 0.0 || rx >= w || ry >= h {
            return None;
        }
        let s = if matches!(self.mode, Mode::Text(_)) { 1.0 } else { f64::from(self.scale) };
        Some((rx / s, ry / s))
    }

    /// Hover at chart position `p` (or nowhere): active elements and tooltip, as Chart.js's
    /// event handling sets them.
    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let (active, tooltip) = match p {
            Some((x, y)) => {
                let a = self.chart.elements_at(x, y, &self.chart.hover_interaction());
                let t = self.chart.elements_at(x, y, &self.chart.tooltip_interaction());
                (a, t)
            }
            None => (vec![], vec![]),
        };
        let changed = active != self.chart.active() || self.pointer != p;
        self.chart.set_active(active);
        self.chart.set_tooltip(tooltip, p);
        self.pointer = p;
        changed
    }

    /// Select data index `i` across datasets (arrow keys), with a tooltip.
    fn select_index(&mut self, i: usize) {
        let els: Vec<ActiveElement> = self
            .chart
            .metas
            .iter()
            .filter(|m| m.visible)
            .filter(|m| i < m.points.len().max(m.bars.len()).max(m.arcs.len()))
            .map(|m| ActiveElement { dataset: m.index, index: i })
            .collect();
        self.chart.set_active(els.clone());
        self.chart.set_tooltip(els, None);
        self.keyed = Some(i);
    }

    fn data_len(&self) -> usize {
        self.chart.metas.iter().map(|m| m.points.len().max(m.bars.len()).max(m.arcs.len())).max().unwrap_or(0)
    }

    /// Handle one input event.
    pub fn handle(&mut self, ev: &Event) -> Action {
        match ev {
            Event::Key(Key::Char('q'), _) | Event::Key(Key::Escape, _) => Action::Quit,
            Event::Key(Key::Char('c'), m) if m.ctrl => Action::Quit,
            Event::Key(Key::Char('r'), _) => {
                self.chart.replay();
                self.dirty = true;
                Action::Redraw
            }
            Event::Key(Key::Char('s'), _) => Action::Save(self.png()),
            Event::Key(Key::Char('l'), _) => {
                let mut cfg = self.config.clone();
                let shown =
                    cfg.options.pointer("/plugins/legend/display").and_then(serde_json::Value::as_bool)
                        != Some(false);
                cfg.options["plugins"]["legend"]["display"] = serde_json::Value::Bool(!shown);
                self.update(cfg);
                Action::Redraw
            }
            Event::Key(Key::Char(d @ '1'..='9'), _) => {
                let ds = d.to_digit(10).unwrap_or(1) as usize - 1;
                if self.chart.chart_type().is_arc() || self.chart.chart_type() == crate::ChartType::PolarArea
                {
                    self.chart.toggle_data_visibility(ds);
                } else if ds < self.config.datasets().len() {
                    let v = self.chart.dataset_visible(ds);
                    self.chart.set_dataset_visibility(ds, !v);
                }
                self.dirty = true;
                Action::Redraw
            }
            Event::Key(k @ (Key::Left | Key::Right | Key::Home | Key::End), _) => {
                let n = self.data_len();
                if n == 0 {
                    return Action::None;
                }
                let cur = self.keyed;
                let i = match k {
                    Key::Left => cur.map_or(n - 1, |c| c.saturating_sub(1)),
                    Key::Right => cur.map_or(0, |c| (c + 1).min(n - 1)),
                    Key::Home => 0,
                    _ => n - 1,
                };
                self.select_index(i);
                self.dirty = true;
                Action::Redraw
            }
            Event::Mouse(m) => {
                let p = self.to_chart(m.x, m.y, m.pixels);
                match (m.kind, m.button) {
                    (MouseKind::Press, Button::Left) => {
                        if let Some((x, y)) = p
                            && self.chart.legend_click(x, y)
                        {
                            self.dirty = true;
                            return Action::Redraw;
                        }
                        Action::None
                    }
                    (MouseKind::Move | MouseKind::Press, _) => {
                        self.keyed = None;
                        if self.hover(p) {
                            self.dirty = true;
                            Action::Redraw
                        } else {
                            Action::None
                        }
                    }
                    _ => Action::None,
                }
            }
            Event::FocusOut => {
                if self.hover(None) {
                    self.dirty = true;
                    return Action::Redraw;
                }
                Action::None
            }
            Event::Resize { cols, rows, px_width, px_height } => {
                let (cw, ch) = (
                    f32::from(*px_width) / f32::from((*cols).max(1)),
                    f32::from(*px_height) / f32::from((*rows).max(1)),
                );
                if cw > 0.0 && ch > 0.0 {
                    self.resize(*cols, *rows, cw, ch);
                } else {
                    self.resize(*cols, *rows, self.info.cell_width, self.info.cell_height);
                }
                Action::Redraw
            }
            _ => Action::None,
        }
    }

    /// The chart as a PNG at its on-screen size.
    pub fn png(&mut self) -> Vec<u8> {
        let (w, h) = self.pixel_size();
        let mut c = SkiaCanvas::with_pixel_size(w.max(1), h.max(1), self.scale);
        if let Some(bg) = self.opts.background {
            c.fill_background(bg);
        }
        let mut chart = Chart::with_theme(self.config.clone(), self.theme());
        chart.dpr = f64::from(self.scale);
        chart.draw(&mut c);
        crate::Image { width: c.pixel_width(), height: c.pixel_height(), rgba: c.to_rgba() }.to_png()
    }

    /// Draw everything again on the next frame (e.g. after the host wrote over the screen).
    pub fn redraw(&mut self) {
        self.dirty = true;
    }

    /// Whether a frame is due (something changed or an animation runs).
    pub fn needs_frame(&self) -> bool {
        self.dirty || self.chart.is_animating()
    }

    /// Render a frame if one is due and the frame interval has passed: the bytes to write
    /// (they leave the cursor where it was). `now` is milliseconds on any monotonic clock.
    pub fn frame(&mut self, now: f64) -> Option<Vec<u8>> {
        if !self.needs_frame() {
            return None;
        }
        if let Some(last) = self.last_frame
            && now - last < self.opts.frame_interval
        {
            return None;
        }
        self.last_frame = Some(now);
        self.dirty = false;
        self.chart.set_time(now);
        let mut out = Vec::new();
        out.extend_from_slice(b"\x1b7"); // save cursor
        if std::mem::take(&mut self.clear) {
            out.extend_from_slice(b"\x1b[2J");
        }
        match self.mode {
            Mode::Text(cell) => self.draw_text(cell, &mut out),
            Mode::Sixel => self.draw_sixel(&mut out),
            Mode::Kitty | Mode::Auto => self.draw_kitty(&mut out),
        }
        if self.opts.status_line {
            self.draw_status(&mut out);
        }
        out.extend_from_slice(b"\x1b8"); // restore cursor
        self.sent = true;
        Some(out)
    }

    fn render_rgba(&mut self) -> (u32, u32, Vec<u8>) {
        let (w, h) = self.pixel_size();
        let (w, h) = (w.max(1), h.max(1));
        let mut c = SkiaCanvas::with_pixel_size(w, h, self.scale);
        if let Some(bg) = self.opts.background {
            c.fill_background(bg);
        }
        self.chart.draw(&mut c);
        (w, h, c.to_rgba())
    }

    fn cursor_to(&self, out: &mut Vec<u8>, col: u16, row: u16) {
        out.extend_from_slice(format!("\x1b[{};{}H", row + 1, col + 1).as_bytes());
    }

    fn draw_kitty(&mut self, out: &mut Vec<u8>) {
        let (w, h, rgba) = self.render_rgba();
        let img = RgbaImage::new(w, h, &rgba);
        let target = self.shown.map_or(self.opts.image_id, kitty::alternate_id);
        let mut t = TransmitOptions::new(target);
        t.placement_id = Some(1);
        t.columns = Some(u32::from(self.region.cols));
        t.rows = Some(u32::from(self.region.rows));
        t.hold_cursor = true;
        t.medium = match &self.opts.medium {
            Medium::Direct => Medium::Direct,
            kind => kittychart_term_write(kind, &rgba).unwrap_or(Medium::Direct),
        };
        if t.medium != Medium::Direct {
            t.compress = false;
        }
        let mut esc = Vec::new();
        kitty::transmit_and_display(img, &t, &mut esc);
        // The new image is complete and placed: now the old one can go.
        if let Some(old) = self.shown.replace(target) {
            kitty::delete_image(old, kitty::Quiet::Silent, &mut esc);
        }
        self.cursor_to(out, self.region.col, self.region.row);
        out.extend_from_slice(&if self.info.tmux { kitty::tmux_passthrough(&esc) } else { esc });
    }

    fn draw_sixel(&mut self, out: &mut Vec<u8>) {
        let (w, h, rgba) = self.render_rgba();
        let background = self.info.colors.as_ref().map(|c| c.background);
        self.cursor_to(out, self.region.col, self.region.row);
        // An opaque background, so each frame fully replaces the last.
        let bg = background.or(Some([0, 0, 0]));
        sixel::encode(
            RgbaImage::new(w, h, &rgba),
            &SixelOptions { background: bg, opaque: true, ..SixelOptions::default() },
            out,
        );
    }

    fn draw_text(&mut self, cell: CellMode, out: &mut Vec<u8>) {
        let (cols, rows) = (u32::from(self.region.cols), u32::from(self.region.rows));
        let mut c = CellCanvas::new(cols, rows, cell);
        self.chart.draw(&mut c);
        let ansi = c.into_grid().to_ansi();
        for (i, line) in ansi.split("\r\n").enumerate() {
            self.cursor_to(out, self.region.col, self.region.row + i as u16);
            out.extend_from_slice(line.as_bytes());
        }
    }

    fn draw_status(&self, out: &mut Vec<u8>) {
        let row = self.region.row + self.region.rows;
        if row >= self.info.rows {
            return;
        }
        self.cursor_to(out, 0, row);
        let hint = " q quit · ←/→ step · 1-9 toggle · l legend · r replay · s save ";
        let n = usize::from(self.info.cols);
        let text: String = hint.chars().take(n).collect();
        out.extend_from_slice(b"\x1b[2K\x1b[2m");
        out.extend_from_slice(text.as_bytes());
        out.extend_from_slice(b"\x1b[0m");
    }

    /// Bytes that free the session's kitty image (write before leaving).
    pub fn cleanup(&self) -> Vec<u8> {
        if !self.sent || self.mode != Mode::Kitty {
            return vec![];
        }
        let mut out = Vec::new();
        kitty::delete_image(self.shown.unwrap_or(self.opts.image_id), kitty::Quiet::Silent, &mut out);
        if self.info.tmux { kitty::tmux_passthrough(&out) } else { out }
    }
}

#[cfg(unix)]
fn kittychart_term_write(kind: &Medium, bytes: &[u8]) -> std::io::Result<Medium> {
    kittychart_term::local::write(kind, bytes)
}

#[cfg(not(unix))]
fn kittychart_term_write(_kind: &Medium, _bytes: &[u8]) -> std::io::Result<Medium> {
    Ok(Medium::Direct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kittychart_term::input::{Mods, Mouse};

    fn info() -> TerminalInfo {
        TerminalInfo {
            cols: 80,
            rows: 25,
            cell_width: 10.0,
            cell_height: 20.0,
            kitty: true,
            sixel: false,
            tmux: false,
            colors: None,
        }
    }

    fn session(mode: Mode) -> Session {
        let config = ChartConfig::from_json(
            r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[{"label":"One","data":[3,9,4]},{"label":"Two","data":[5,2,7]}]},"options":{"interaction":{"mode":"index"}}}"#,
        )
        .unwrap();
        Session::new(config, info(), SessionOptions { mode, image_id: 7, ..SessionOptions::default() })
    }

    fn mouse(x: u32, y: u32) -> Event {
        Event::Mouse(Mouse {
            kind: MouseKind::Move,
            button: Button::None,
            x,
            y,
            mods: Mods::default(),
            pixels: true,
        })
    }

    #[test]
    fn frames_only_when_something_changed() {
        let mut s = session(Mode::Kitty);
        let f = s.frame(0.0).expect("first frame");
        assert!(f.starts_with(b"\x1b7\x1b[1;1H\x1b_Ga=T,f=32,s=800,v=480,i=7,p=1,c=80,r=24,o=z,C=1"));
        // The intro animation runs for 1 s, frames at most every 33 ms.
        assert!(s.frame(10.0).is_none());
        assert!(s.frame(40.0).is_some());
        assert!(s.frame(2000.0).is_some()); // the last animation frame
        assert!(s.frame(3000.0).is_none(), "idle: nothing to send");
    }

    /// Each kitty frame goes out under the other id and then frees the one it replaces (one
    /// id re-sent in chunks shows nothing until the last chunk: a flicker).
    #[test]
    fn kitty_frames_alternate_ids() {
        let mut s = session(Mode::Kitty);
        let alt = kitty::alternate_id(7);
        let f1 = s.frame(0.0).unwrap();
        assert!(f1.windows(4).any(|w| w == b"i=7,") && !f1.windows(4).any(|w| w == b"a=d,"));
        let f2 = String::from_utf8_lossy(&s.frame(40.0).unwrap()).into_owned();
        let (send, free) = (f2.find(&format!("i={alt},")).unwrap(), f2.find("a=d,d=I,i=7,").unwrap());
        assert!(send < free, "the new image goes out before the old one is freed");
        let f3 = String::from_utf8_lossy(&s.frame(80.0).unwrap()).into_owned();
        assert!(f3.contains("i=7,") && f3.contains(&format!("a=d,d=I,i={alt},")));
        assert!(String::from_utf8_lossy(&s.cleanup()).contains("i=7,"), "frees the image on screen");
    }

    /// After a resize the next frame clears the screen first (old sixel pixels or a status line
    /// left at the old size would remain), once.
    #[test]
    fn resize_clears_the_screen_once() {
        let mut s = session(Mode::Sixel);
        assert!(!contains(&s.frame(0.0).unwrap(), b"\x1b[2J"));
        s.resize(60, 20, 10.0, 20.0);
        assert!(s.frame(40.0).unwrap().starts_with(b"\x1b7\x1b[2J"));
        assert!(!contains(&s.frame(80.0).unwrap(), b"\x1b[2J"));
    }

    fn contains(hay: &[u8], needle: &[u8]) -> bool {
        hay.windows(needle.len()).any(|w| w == needle)
    }

    #[test]
    fn hover_sets_active_elements_and_tooltip() {
        let mut s = session(Mode::Kitty);
        s.frame(0.0);
        s.frame(5000.0);
        // Middle of the screen (pixels): the second category.
        assert_eq!(s.handle(&mouse(400, 300)), Action::Redraw);
        assert_eq!(s.chart().active().len(), 2, "index mode: both datasets");
        assert!(s.frame(5100.0).is_some());
        assert!(s.chart().tooltip.is_some());
        assert_eq!(s.handle(&mouse(400, 300)), Action::None, "same place: nothing to do");
        assert_eq!(s.handle(&Event::FocusOut), Action::Redraw);
        assert!(s.chart().active().is_empty());
    }

    #[test]
    fn keys_step_toggle_and_quit() {
        let mut s = session(Mode::Text(CellMode::Auto));
        s.frame(0.0);
        let k = |c| Event::Key(c, Mods::default());
        assert_eq!(s.handle(&k(Key::Right)), Action::Redraw);
        assert_eq!(s.chart().active()[0].index, 0);
        s.handle(&k(Key::Right));
        assert_eq!(s.chart().active()[0].index, 1);
        s.handle(&k(Key::Char('2')));
        assert!(!s.chart().dataset_visible(1));
        assert!(matches!(s.handle(&k(Key::Char('s'))), Action::Save(png) if png.starts_with(b"\x89PNG")));
        assert_eq!(s.handle(&k(Key::Char('q'))), Action::Quit);
        let f = s.frame(10_000.0).unwrap();
        assert!(String::from_utf8_lossy(&f).contains("One"), "text frames draw cells");
    }
}
