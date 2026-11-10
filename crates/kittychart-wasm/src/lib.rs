//! KittyChart in the browser, for the demo site: the same library on both sides of the page.
//!
//! - [`WebChart`] draws a chart into RGBA pixels for a `<canvas>`, with hover, tooltips,
//!   legend clicks and animation (`Chart` + the Skia canvas, as the PNG renderer uses).
//! - [`Detect`], [`Viewer`] and [`inline`] are the terminal side. They take bytes from a
//!   terminal and return bytes for it, exactly as the CLI does over a TTY: capability
//!   detection ([`kittychart::term::detect::Detector`]), `kittychart view`
//!   ([`kittychart::session::Session`] fed by [`kittychart::term::input::Parser`]) and the
//!   inline `kittychart FILE` ([`kittychart::show::prepare_for`]).
//!
//! Nothing here knows about the browser terminal it talks to (restty on the demo page).

use kittychart::canvas::SkiaCanvas;
use kittychart::session::{Action, Session, SessionOptions};
use kittychart::show::{Mode, ShowOptions, TerminalInfo};
use kittychart::term::detect::{Capabilities, Detector};
use kittychart::term::input::Parser;
use kittychart::{Chart, ChartConfig, Theme};
use kittychart_core::interaction::{ActiveElement, tooltip_position};
use wasm_bindgen::prelude::*;

fn config(json: &str) -> Result<ChartConfig, JsError> {
    ChartConfig::from_json(json).map_err(|e| JsError::new(&e.to_string()))
}

fn mode(protocol: &str) -> Result<Mode, JsError> {
    protocol.parse().map_err(|e: String| JsError::new(&e))
}

/// Active elements as flat `[dataset, index, dataset, index, …]` (cheap to pass to JS).
fn flatten(els: &[ActiveElement]) -> Vec<u32> {
    els.iter().flat_map(|e| [e.dataset as u32, e.index as u32]).collect()
}

fn unflatten(pairs: &[u32]) -> Vec<ActiveElement> {
    pairs
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&[d, i]| ActiveElement { dataset: d as usize, index: i as usize })
        .collect()
}

/// A chart drawn into pixels for a `<canvas>`.
#[wasm_bindgen]
#[derive(Debug)]
pub struct WebChart {
    chart: Chart,
    width: f32,
    height: f32,
    dpr: f32,
    dirty: bool,
    rgba: Vec<u8>,
    pixel_size: (u32, u32),
}

#[wasm_bindgen]
impl WebChart {
    /// A chart of `width`×`height` CSS pixels at `dpr` device pixels per CSS pixel, in
    /// Chart.js's default colors, or in colors derived from a terminal's (`theme`).
    #[wasm_bindgen(constructor)]
    pub fn new(
        json: &str,
        width: f32,
        height: f32,
        dpr: f32,
        theme: Option<TermInfo>,
    ) -> Result<WebChart, JsError> {
        let theme = theme.and_then(|t| t.info.colors.as_ref().map(Theme::from_terminal));
        let mut chart = Chart::with_theme(config(json)?, theme);
        chart.dpr = f64::from(dpr);
        Ok(WebChart { chart, width, height, dpr, dirty: true, rgba: Vec::new(), pixel_size: (0, 0) })
    }

    pub fn resize(&mut self, width: f32, height: f32, dpr: f32) {
        (self.width, self.height, self.dpr) = (width, height, dpr);
        self.chart.dpr = f64::from(dpr);
        self.dirty = true;
    }

    /// New data or options: animate to them.
    pub fn update(&mut self, json: &str) -> Result<(), JsError> {
        self.chart.update(config(json)?);
        self.dirty = true;
        Ok(())
    }

    /// Play the opening animation again.
    pub fn replay(&mut self) {
        self.chart.replay();
        self.dirty = true;
    }

    /// The pointer moved to `(x, y)` CSS pixels.
    pub fn pointer(&mut self, x: f64, y: f64) {
        self.dirty |= self.chart.hover_at(Some((x, y)));
    }

    /// The pointer left the canvas.
    pub fn leave(&mut self) {
        self.dirty |= self.chart.hover_at(None);
    }

    /// A click at `(x, y)`: toggles the legend item there. Returns the item's number (the
    /// dataset, or for pie, doughnut and polar area charts the data index; the viewer's `1`–`9`
    /// keys toggle the same), or -1 when no item was hit.
    pub fn click(&mut self, x: f64, y: f64) -> i32 {
        let Some(item) = self.chart.legend_item_at(x, y).map(|i| i.index.unwrap_or(i.dataset_index)) else {
            return -1;
        };
        self.chart.legend_click(x, y);
        self.dirty = true;
        item as i32
    }

    /// Toggle legend item `item` as the viewer's number keys do: a dataset, or for pie,
    /// doughnut and polar area charts a data index.
    pub fn toggle(&mut self, item: usize) {
        let t = self.chart.chart_type();
        if t.is_arc() || t == kittychart::ChartType::PolarArea {
            self.chart.toggle_data_visibility(item);
        } else {
            let v = self.chart.dataset_visible(item);
            self.chart.set_dataset_visibility(item, !v);
        }
        self.dirty = true;
    }

    /// Whether `(x, y)` is over a legend item (for the pointer cursor).
    pub fn over_legend(&self, x: f64, y: f64) -> bool {
        self.chart.legend_item_at(x, y).is_some()
    }

    /// The hovered elements, `[dataset, index, …]`.
    pub fn active(&self) -> Vec<u32> {
        flatten(self.chart.active())
    }

    /// Show `[dataset, index, …]` as hovered, with their tooltip (mirroring another view).
    pub fn highlight(&mut self, pairs: &[u32]) {
        let els = unflatten(pairs);
        if els.as_slice() != self.chart.active() {
            self.chart.set_active(els.clone());
            self.chart.set_tooltip(els, None);
            self.dirty = true;
        }
    }

    /// Draw if anything changed or an animation runs. Returns whether new pixels are ready.
    pub fn frame(&mut self, now: f64) -> bool {
        if !self.dirty && !self.chart.is_animating() {
            return false;
        }
        self.dirty = false;
        self.chart.set_time(now);
        let mut c = SkiaCanvas::new(self.width, self.height, self.dpr);
        self.chart.draw(&mut c);
        self.pixel_size = (c.pixel_width(), c.pixel_height());
        self.rgba = c.to_rgba();
        true
    }

    pub fn pixel_width(&self) -> u32 {
        self.pixel_size.0
    }

    pub fn pixel_height(&self) -> u32 {
        self.pixel_size.1
    }

    /// The last frame's RGBA pixels (straight alpha), `pixel_width × pixel_height × 4` bytes
    /// starting here in wasm memory: read them without copying.
    pub fn pixels_ptr(&self) -> *const u8 {
        self.rgba.as_ptr()
    }

    pub fn pixels_len(&self) -> usize {
        self.rgba.len()
    }
}

/// Terminal capability detection over any byte channel: KittyChart's own
/// [`Detector`], plus one allowance for the demo's terminal.
///
/// The detector takes the DA1 reply as proof that every earlier query has been answered, as
/// the kitty graphics protocol recommends. restty 0.3.0 answers DA1 (and the cell-size and
/// color queries) in its JavaScript layer as soon as it sees them, before its VT core
/// answers the kitty `a=q` probe sent ahead of them. So this wrapper keeps watching for the
/// probe's `OK` after DA1, for as long as the host keeps feeding (the demo waits up to 500 ms), and
/// [`Detect::late_kitty`] says whether that is how kitty support was found.
#[wasm_bindgen]
#[derive(Debug, Default)]
pub struct Detect {
    detector: Detector,
    caps: Option<Capabilities>,
    /// Everything fed so far (replies are small), to find a late kitty reply.
    seen: Vec<u8>,
    late_kitty: bool,
}

#[wasm_bindgen]
impl Detect {
    #[wasm_bindgen(constructor)]
    pub fn new() -> Detect {
        Detect::default()
    }

    /// The queries to write to the terminal.
    pub fn queries(&self) -> Vec<u8> {
        self.detector.queries()
    }

    /// Bytes the terminal sent back. Returns `true` once DA1 has arrived (keep feeding briefly
    /// to catch a late kitty reply).
    pub fn feed(&mut self, bytes: &[u8]) -> bool {
        if self.seen.len() < 1 << 16 {
            self.seen.extend_from_slice(bytes);
        }
        if self.caps.is_none()
            && let Some(c) = self.detector.feed(bytes)
        {
            self.caps = Some(c.clone());
        }
        if let Some(caps) = self.caps.as_mut()
            && !caps.kitty_graphics
        {
            let ok = format!("\x1b_Gi={};OK", kittychart::term::detect::KITTY_PROBE_ID);
            if self.seen.windows(ok.len()).any(|w| w == ok.as_bytes()) {
                caps.kitty_graphics = true;
                self.late_kitty = true;
            }
        }
        self.caps.is_some()
    }

    /// Whether kitty support was only found by the reply arriving after DA1.
    pub fn late_kitty(&self) -> bool {
        self.late_kitty
    }

    /// What was found, for a terminal of `cols`×`rows` cells of `cell_width`×`cell_height`
    /// device pixels (pass 0 to use the terminal's own cell-size report). Without complete
    /// answers (a timeout), the terminal counts as text-only.
    pub fn info(&self, cols: u16, rows: u16, cell_width: u16, cell_height: u16) -> TermInfo {
        let cell = (cell_width > 0 && cell_height > 0).then_some((cell_width, cell_height));
        TermInfo { info: TerminalInfo::from_capabilities(cols, rows, cell, self.caps.as_ref(), false) }
    }
}

/// What a terminal supports (from [`Detect::info`]).
#[wasm_bindgen]
#[derive(Debug, Clone)]
pub struct TermInfo {
    info: TerminalInfo,
}

#[wasm_bindgen]
impl TermInfo {
    pub fn kitty(&self) -> bool {
        self.info.kitty
    }
    pub fn sixel(&self) -> bool {
        self.info.sixel
    }
    pub fn colors(&self) -> bool {
        self.info.colors.is_some()
    }
    pub fn palette(&self) -> bool {
        self.info.colors.as_ref().is_some_and(|c| c.palette.is_some())
    }
    pub fn cols(&self) -> u16 {
        self.info.cols
    }
    pub fn rows(&self) -> u16 {
        self.info.rows
    }
    pub fn cell_width(&self) -> f32 {
        self.info.cell_width
    }
    pub fn cell_height(&self) -> f32 {
        self.info.cell_height
    }
    /// The mode `auto` resolves to: `kitty`, `sixel` or `text`.
    pub fn best(&self) -> String {
        match self.info.best_mode() {
            Mode::Kitty => "kitty",
            Mode::Sixel => "sixel",
            _ => "text",
        }
        .into()
    }
}

/// `kittychart FILE [--protocol P] [--cols N]`: the bytes the CLI writes for an inline chart
/// (they end with a newline below it).
#[wasm_bindgen]
pub fn inline(
    json: &str,
    term: &TermInfo,
    protocol: &str,
    cols: Option<u16>,
    placeholders: bool,
) -> Result<Vec<u8>, JsError> {
    let opts = ShowOptions { cols, placeholders, ..ShowOptions::default() };
    Ok(kittychart::show::prepare_for(config(json)?, &term.info, mode(protocol)?, &opts).bytes)
}

/// `kittychart view FILE`: an interactive chart filling the terminal.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Viewer {
    session: Session,
    parser: Parser,
    saved: Option<Vec<u8>>,
}

/// What [`Viewer::input`] asks of the host.
#[wasm_bindgen]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewerAction {
    None = 0,
    Redraw = 1,
    Quit = 2,
    /// A PNG is ready: [`Viewer::take_png`].
    Save = 3,
}

#[wasm_bindgen]
impl Viewer {
    #[wasm_bindgen(constructor)]
    pub fn new(json: &str, term: &TermInfo, protocol: &str) -> Result<Viewer, JsError> {
        let opts = SessionOptions { mode: mode(protocol)?, ..SessionOptions::default() };
        Ok(Viewer {
            session: Session::new(config(json)?, term.info.clone(), opts),
            parser: Parser::new(true),
            saved: None,
        })
    }

    /// The bytes that set up the screen (alternate screen, mouse and focus reporting) and,
    /// [`Viewer::leave`], restore it: the same as the CLI's.
    pub fn enter() -> Vec<u8> {
        kittychart::term::input::ENABLE.as_bytes().to_vec()
    }

    /// Bytes the terminal sent (keys, mouse reports, focus changes). Returns the strongest
    /// action among the events they hold.
    pub fn input(&mut self, bytes: &[u8]) -> ViewerAction {
        let mut out = ViewerAction::None;
        for ev in self.parser.feed(bytes) {
            let a = match self.session.handle(&ev) {
                Action::None => ViewerAction::None,
                Action::Redraw => ViewerAction::Redraw,
                Action::Quit => ViewerAction::Quit,
                Action::Save(png) => {
                    self.saved = Some(png);
                    ViewerAction::Save
                }
            };
            if a as u8 > out as u8 {
                out = a;
            }
        }
        out
    }

    /// The PNG from the last `s` key.
    pub fn take_png(&mut self) -> Option<Vec<u8>> {
        self.saved.take()
    }

    /// A frame to write, if one is due (`now` in milliseconds on any monotonic clock).
    pub fn frame(&mut self, now: f64) -> Option<Vec<u8>> {
        self.session.frame(now)
    }

    pub fn needs_frame(&self) -> bool {
        self.session.needs_frame()
    }

    /// New data or options: animate to them.
    pub fn update(&mut self, json: &str) -> Result<(), JsError> {
        self.session.update(config(json)?);
        Ok(())
    }

    pub fn resize(&mut self, cols: u16, rows: u16, cell_width: f32, cell_height: f32) {
        self.session.resize(cols, rows, cell_width, cell_height);
    }

    /// Draw everything on the next frame (after something else wrote over the screen).
    pub fn redraw(&mut self) {
        self.session.redraw();
    }

    /// The hovered elements, `[dataset, index, …]`.
    pub fn active(&self) -> Vec<u32> {
        flatten(self.session.chart().active())
    }

    /// The SGR pixel pointer report (mode 1016) a terminal would send for a pointer over
    /// element `(dataset, index)`, or over nothing when it isn't drawn. Feeding it to
    /// [`Viewer::input`] hovers that element as a real pointer would.
    pub fn pointer_report(&self, dataset: u32, index: u32) -> Option<String> {
        let e = ActiveElement { dataset: dataset as usize, index: index as usize };
        let (x, y) = tooltip_position(self.session.chart(), e)?;
        let (px, py) = self.session.to_terminal(x, y);
        Some(format!("\x1b[<35;{};{}M", px + 1, py + 1))
    }

    /// Bytes that free the chart's image: write before leaving.
    pub fn cleanup(&self) -> Vec<u8> {
        self.session.cleanup()
    }

    /// The bytes that restore the screen.
    pub fn leave() -> Vec<u8> {
        kittychart::term::input::DISABLE.as_bytes().to_vec()
    }

    /// The output mode in use: `kitty`, `sixel` or `text`.
    pub fn mode(&self) -> String {
        match self.session.mode() {
            Mode::Kitty | Mode::Auto => "kitty",
            Mode::Sixel => "sixel",
            Mode::Text(_) => "text",
        }
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KITTY_OK: &[u8] = b"\x1b_Gi=19267;OK\x1b\\";
    const DA1: &[u8] = b"\x1b[?1;2c";

    #[test]
    fn detects_kitty_in_order() {
        let mut d = Detect::new();
        assert!(d.feed(&[KITTY_OK, DA1].concat()));
        let info = d.info(80, 24, 10, 20);
        assert!(info.kitty() && !d.late_kitty());
    }

    #[test]
    fn detects_a_kitty_reply_that_arrives_after_da1() {
        // restty's order: DA1 first, then the kitty reply (sometimes in a later read).
        let mut d = Detect::new();
        assert!(d.feed(DA1));
        assert!(!d.info(80, 24, 10, 20).kitty());
        assert!(d.feed(&KITTY_OK[..7]));
        assert!(d.feed(&KITTY_OK[7..]));
        assert!(d.info(80, 24, 10, 20).kitty() && d.late_kitty());
    }

    #[test]
    fn no_kitty_reply_means_no_kitty() {
        let mut d = Detect::new();
        assert!(d.feed(b"\x1b[6;20;10t\x1b[?62;4c"));
        let info = d.info(80, 24, 0, 0);
        assert!(!info.kitty() && info.sixel() && !d.late_kitty());
        assert_eq!((info.cell_width(), info.cell_height()), (10.0, 20.0));
    }

    #[test]
    fn pointer_reports_hover_the_element() {
        let json = r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[{"data":[3,9,4]}]},"options":{"animation":false}}"#;
        let mut d = Detect::new();
        d.feed(&[KITTY_OK, DA1].concat());
        let mut v = Viewer::new(json, &d.info(80, 25, 10, 20), "kitty").unwrap();
        v.frame(0.0).expect("first frame");
        let report = v.pointer_report(0, 1).expect("bar 1 is drawn");
        assert_eq!(v.input(report.as_bytes()), ViewerAction::Redraw);
        assert_eq!(v.active(), [0, 1]);
    }
}
