//! A [ratatui](https://ratatui.rs) widget that draws Chart.js charts with KittyChart.
//!
//! With kitty graphics, the chart is a real image shown through **Unicode placeholders**: the
//! widget fills its area with placeholder cells, and the image travels separately. It therefore
//! moves, scrolls and clips with the text grid like any other widget, and works inside tmux.
//! Without graphics, the chart is drawn with block and Braille characters.
//!
//! The widget never writes to the terminal itself (ratatui widgets only fill a buffer). After
//! drawing, send the image with [`ChartState::take_pending`], and draw again while
//! [`ChartState::needs_redraw`] says so:
//!
//! ```no_run
//! # use kittychart_ratatui::{ChartState, ChartWidget, Graphics};
//! # fn demo(terminal: &mut ratatui::DefaultTerminal, config: &kittychart::ChartConfig) -> std::io::Result<()> {
//! use std::io::Write;
//! let mut state = ChartState::new(1);
//! let graphics = Graphics::Kitty { cell_width: 10.0, cell_height: 20.0 };
//! loop {
//!     terminal.draw(|f| {
//!         f.render_stateful_widget(ChartWidget::new(config, graphics), f.area(), &mut state)
//!     })?;
//!     if let Some(bytes) = state.take_pending() {
//!         std::io::stdout().write_all(&bytes)?;
//!         std::io::stdout().flush()?;
//!     }
//!     if !state.needs_redraw() {
//!         break;
//!     }
//! }
//! # Ok(()) }
//! ```
//!
//! The image is only re-sent when the chart, its size, the colors or the hover state change.
//! A changed image goes out under a second image id while the placeholders still show the
//! current one; the next draw switches them over and frees the old image. (Re-sending under
//! the same id makes the terminal drop the image until the transfer completes, which flickers.)
//! So each state uses two ids: `image_id` and [`kitty::alternate_id`] of it.

use std::hash::{DefaultHasher, Hash, Hasher};

use kittychart::{Chart, ChartConfig, TerminalColors};
use kittychart_canvas::SkiaCanvas;
use kittychart_canvas::cells::{CELL_H, CELL_W, CellCanvas, CellMode};
use kittychart_term::RgbaImage;
use kittychart_term::kitty::{self, TransmitOptions};
use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::{Color, Modifier, Style};
use ratatui_core::widgets::StatefulWidget;

/// How the widget draws.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Graphics {
    /// kitty graphics through Unicode placeholders. Cell size in device pixels (from
    /// `TIOCGWINSZ` or `CSI 16 t`; see `kittychart_term::tty`).
    Kitty { cell_width: f32, cell_height: f32 },
    /// Text cells: block and Braille characters.
    Cells(CellMode),
}

/// A Chart.js chart.
#[derive(Debug, Clone)]
pub struct ChartWidget<'a> {
    config: &'a ChartConfig,
    graphics: Graphics,
    colors: Option<TerminalColors>,
    tmux: bool,
}

impl<'a> ChartWidget<'a> {
    pub fn new(config: &'a ChartConfig, graphics: Graphics) -> Self {
        Self { config, graphics, colors: None, tmux: false }
    }

    /// Derive the chart's default colors from the terminal's (see `kittychart::Theme`).
    /// Without them, Chart.js's defaults for white pages are used.
    pub fn colors(mut self, colors: Option<TerminalColors>) -> Self {
        self.colors = colors;
        self
    }

    /// Wrap the image for tmux passthrough (tmux needs `set -g allow-passthrough on`).
    pub fn tmux(mut self, tmux: bool) -> Self {
        self.tmux = tmux;
        self
    }

    fn key(&self, area: Rect) -> u64 {
        let mut h = DefaultHasher::new();
        format!("{:?}", self.config.chart_type).hash(&mut h);
        self.config.data.to_string().hash(&mut h);
        self.config.options.to_string().hash(&mut h);
        (area.width, area.height).hash(&mut h);
        format!("{:?}{:?}{}", self.graphics, self.colors, self.tmux).hash(&mut h);
        h.finish()
    }
}

/// Per-widget state: the kitty image id, what has been sent, and the live chart (hover,
/// tooltip, legend toggles and animation live here between frames).
#[derive(Debug)]
pub struct ChartState {
    image_id: u32,
    sent: Option<u64>,
    pending: Option<Vec<u8>>,
    /// Double buffering (see [`kitty::alternate_id`]): the id the placeholders show, a frame
    /// sent under the other id that the next draw switches to, and an id to delete once the
    /// placeholders no longer show it.
    shown: Option<u32>,
    incoming: Option<u32>,
    ready: Option<u32>,
    /// A change came in during a switching draw (which sends nothing): draw again.
    again: bool,
    /// The live chart, and what it was built for (chart type, colors, graphics).
    chart: Option<(u64, Chart)>,
    /// The config the chart shows (to animate to a changed one).
    config_key: Option<u64>,
    /// Where it was last drawn, and the scale (device px per CSS px; 1 in cell mode).
    area: Rect,
    cell: (f32, f32),
    scale: f32,
    cells: bool,
    pointer: Option<(f64, f64)>,
    now: Option<f64>,
}

impl ChartState {
    /// `image_id` must be unique among the images your app shows (1..=0xFFFFFF). The widget
    /// also uses its partner `image_id | 1 << 24` ([`kitty::alternate_id`]) for double buffering.
    pub fn new(image_id: u32) -> Self {
        Self {
            image_id: image_id.clamp(1, 0x00FF_FFFF),
            sent: None,
            pending: None,
            shown: None,
            incoming: None,
            ready: None,
            again: false,
            chart: None,
            config_key: None,
            area: Rect::default(),
            cell: (8.0, 16.0),
            scale: 1.0,
            cells: false,
            pointer: None,
            now: None,
        }
    }

    pub fn image_id(&self) -> u32 {
        self.image_id
    }

    /// Escape sequences to write to the terminal after drawing (the image, when it changed).
    ///
    /// A changed image goes out under a second id while the placeholders keep showing the
    /// current one, so nothing flickers while it transfers; [`ChartState::needs_redraw`] then
    /// asks for one more draw, which switches the placeholders over and frees the old image.
    pub fn take_pending(&mut self) -> Option<Vec<u8>> {
        let p = self.pending.take();
        if p.is_some() {
            self.ready = self.incoming.take().or(self.ready);
        }
        p
    }

    /// Whether to draw again right away: a new frame was sent and the placeholders should
    /// switch to it.
    pub fn needs_redraw(&self) -> bool {
        self.ready.is_some() || self.again
    }

    /// Escapes that free this widget's images in the terminal (write them on exit; images
    /// otherwise stay in the terminal's memory after the placeholders are gone).
    pub fn delete_image(&mut self, tmux: bool) -> Vec<u8> {
        self.sent = None;
        (self.shown, self.incoming, self.ready, self.again) = (None, None, None, false);
        let mut out = Vec::new();
        kitty::delete_image(self.image_id, kitty::Quiet::Silent, &mut out);
        kitty::delete_image(kitty::alternate_id(self.image_id), kitty::Quiet::Silent, &mut out);
        if tmux { kitty::tmux_passthrough(&out) } else { out }
    }

    /// Forget what was sent, e.g. after the terminal was cleared or images deleted.
    pub fn invalidate(&mut self) {
        self.sent = None;
        (self.shown, self.incoming, self.ready, self.again) = (None, None, None, false);
    }

    /// The live chart, once drawn.
    pub fn chart(&self) -> Option<&Chart> {
        self.chart.as_ref().map(|(_, c)| c)
    }

    /// Terminal cell (absolute column and row, as crossterm reports mouse events) → chart CSS
    /// pixels, if inside the widget.
    fn to_chart(&self, column: u16, row: u16) -> Option<(f64, f64)> {
        let a = self.area;
        if column < a.x || row < a.y || column >= a.x + a.width || row >= a.y + a.height {
            return None;
        }
        let (cw, ch) = if self.cells {
            (f64::from(CELL_W), f64::from(CELL_H))
        } else {
            (f64::from(self.cell.0), f64::from(self.cell.1))
        };
        let x = (f64::from(column - a.x) + 0.5) * cw / f64::from(self.scale);
        let y = (f64::from(row - a.y) + 0.5) * ch / f64::from(self.scale);
        Some((x, y))
    }

    /// The pointer moved to `(column, row)`: hover and tooltip, as Chart.js does on mousemove.
    /// Returns whether the chart needs redrawing.
    pub fn handle_mouse(&mut self, column: u16, row: u16) -> bool {
        let p = self.to_chart(column, row);
        self.hover(p)
    }

    /// The pointer left the widget (or the window lost focus).
    pub fn clear_hover(&mut self) -> bool {
        self.hover(None)
    }

    fn hover(&mut self, p: Option<(f64, f64)>) -> bool {
        let Some((_, chart)) = &mut self.chart else { return false };
        let (active, tooltip) = match p {
            Some((x, y)) => (
                chart.elements_at(x, y, &chart.hover_interaction()),
                chart.elements_at(x, y, &chart.tooltip_interaction()),
            ),
            None => (vec![], vec![]),
        };
        let changed = active.as_slice() != chart.active() || self.pointer != p;
        chart.set_active(active);
        chart.set_tooltip(tooltip, p);
        self.pointer = p;
        changed
    }

    /// A click at `(column, row)`: toggles the legend item there. Returns whether it did.
    pub fn handle_click(&mut self, column: u16, row: u16) -> bool {
        let Some((x, y)) = self.to_chart(column, row) else { return false };
        self.chart.as_mut().is_some_and(|(_, c)| c.legend_click(x, y))
    }

    /// Advance the animation clock (milliseconds, monotonic). Without it nothing animates.
    pub fn set_time(&mut self, now_ms: f64) {
        self.now = Some(now_ms);
    }

    /// Whether the chart is mid-animation (draw again soon).
    pub fn is_animating(&self) -> bool {
        self.chart.as_ref().is_some_and(|(_, c)| c.is_animating())
    }

    /// The chart for `widget`: reused while its type, colors and graphics stay the same (a new
    /// config animates to the new data).
    fn chart_for(&mut self, widget: &ChartWidget<'_>) -> &mut Chart {
        let build_key = {
            let mut h = DefaultHasher::new();
            format!("{:?}{:?}{:?}", widget.config.chart_type, widget.colors, widget.graphics).hash(&mut h);
            h.finish()
        };
        let config_key = {
            let mut h = DefaultHasher::new();
            widget.config.data.to_string().hash(&mut h);
            widget.config.options.to_string().hash(&mut h);
            h.finish()
        };
        let theme = widget.colors.as_ref().map(kittychart::Theme::from_terminal);
        let rebuild = self.chart.as_ref().is_none_or(|(k, _)| *k != build_key);
        if rebuild {
            let chart = match widget.graphics {
                Graphics::Cells(_) => Chart::for_cells(widget.config.clone(), theme),
                Graphics::Kitty { .. } => Chart::with_theme(widget.config.clone(), theme),
            };
            self.chart = Some((build_key, chart));
            self.config_key = Some(config_key);
        } else if self.config_key != Some(config_key) {
            if let Some((_, c)) = &mut self.chart {
                c.update(widget.config.clone());
            }
            self.config_key = Some(config_key);
        }
        let now = self.now;
        let (_, chart) = self.chart.as_mut().expect("built above");
        if let Some(t) = now {
            chart.set_time(t);
        }
        chart
    }
}

impl StatefulWidget for ChartWidget<'_> {
    type State = ChartState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut ChartState) {
        let area = area.intersection(buf.area);
        if area.is_empty() {
            return;
        }
        match self.graphics {
            Graphics::Kitty { cell_width, cell_height } => {
                // Placeholder rows are numbered by diacritics (297 of them); taller areas keep
                // their extra rows empty.
                let area = Rect { height: area.height.min(kitty::MAX_PLACEHOLDER_ROWS as u16), ..area };
                let scale = kittychart::terminal::default_scale(cell_height);
                (state.area, state.cell, state.scale, state.cells) =
                    (area, (cell_width, cell_height), scale, false);
                let tmux = self.tmux;
                let key = self.key(area);
                let prev_sent = state.sent;
                let pointer = state.pointer;
                // A frame sent after the last draw: show it now, and free the one it replaces
                // (after this draw is flushed, when no placeholder shows it any more).
                let mut retire = None;
                let switching = state.ready.is_some();
                if let Some(id) = state.ready.take() {
                    retire = state.shown.replace(id);
                }
                // The first image goes straight to the placeholders (nothing to flicker); later
                // ones to the other id.
                let target = state.shown.map_or(state.image_id, kitty::alternate_id);
                let chart = state.chart_for(&self);
                // Something to show that wasn't sent: a new chart, size, hover or legend state,
                // or an animation frame.
                let frame_key = {
                    let mut h = DefaultHasher::new();
                    key.hash(&mut h);
                    format!(
                        "{:?}{:?}{:?}",
                        chart.active(),
                        chart.legend.items.iter().map(|i| i.hidden).collect::<Vec<_>>(),
                        pointer,
                    )
                    .hash(&mut h);
                    h.finish()
                };
                // The draw that switches images sends nothing new (the next one will), which
                // keeps an animation to one frame per draw.
                let due = prev_sent != Some(frame_key) || chart.is_animating();
                let frame = (due && !switching).then(|| {
                    let (w, h) = (
                        (f32::from(area.width) * cell_width).round() as u32,
                        (f32::from(area.height) * cell_height).round() as u32,
                    );
                    let mut canvas = SkiaCanvas::with_pixel_size(w.max(1), h.max(1), scale);
                    chart.dpr = f64::from(scale);
                    chart.draw(&mut canvas);
                    let rgba = canvas.to_rgba();
                    let mut t = TransmitOptions::new(target);
                    t.columns = Some(u32::from(area.width));
                    t.rows = Some(u32::from(area.height));
                    let mut esc = Vec::new();
                    kitty::transmit_virtual(
                        RgbaImage::new(canvas.pixel_width(), canvas.pixel_height(), &rgba),
                        &t,
                        &mut esc,
                    );
                    esc
                });
                // A non-animation change (hover, resize) mustn't wait for the next event.
                state.again = switching && prev_sent != Some(frame_key);
                let mut esc = Vec::new();
                if let Some(old) = retire {
                    kitty::delete_image(old, kitty::Quiet::Silent, &mut esc);
                }
                if let Some(f) = frame {
                    esc.extend_from_slice(&f);
                    state.sent = Some(frame_key);
                    if state.shown.is_none() {
                        state.shown = Some(target);
                    } else {
                        state.incoming = Some(target);
                    }
                }
                if !esc.is_empty() {
                    state.pending = Some(if tmux { kitty::tmux_passthrough(&esc) } else { esc });
                }
                let shown = state.shown.unwrap_or(state.image_id);
                let (r, g, b) = kitty::placeholder_fg(shown);
                let style = Style::new().fg(Color::Rgb(r, g, b));
                for row in 0..area.height {
                    for col in 0..area.width {
                        if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                            cell.set_symbol(&kitty::placeholder_cell(u32::from(row), u32::from(col), shown))
                                .set_style(style);
                        }
                    }
                }
            }
            Graphics::Cells(mode) => {
                (state.area, state.scale, state.cells) = (area, 1.0, true);
                let chart = state.chart_for(&self);
                let mut canvas = CellCanvas::new(u32::from(area.width), u32::from(area.height), mode);
                chart.draw(&mut canvas);
                let grid = canvas.into_grid();
                for row in 0..area.height {
                    for col in 0..area.width {
                        let c = grid.get(u32::from(col), u32::from(row));
                        let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) else { continue };
                        let mut style = Style::new();
                        if let Some([r, g, b]) = c.fg {
                            style = style.fg(Color::Rgb(r, g, b));
                        }
                        if let Some([r, g, b]) = c.bg {
                            style = style.bg(Color::Rgb(r, g, b));
                        }
                        if c.bold {
                            style = style.add_modifier(Modifier::BOLD);
                        }
                        cell.set_char(c.ch).set_style(style);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ChartConfig {
        ChartConfig::from_json(
            r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[{"label":"x","data":[1,3,2]}]}}"#,
        )
        .unwrap()
    }

    #[test]
    fn kitty_fills_area_with_placeholders_and_sends_once() {
        let cfg = config();
        let area = Rect::new(2, 1, 20, 6);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 10));
        let mut state = ChartState::new(0x0001_0203);
        let g = Graphics::Kitty { cell_width: 8.0, cell_height: 16.0 };
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);

        let cell = &buf[(2 + 3, 1 + 2)];
        let mut chars = cell.symbol().chars();
        assert_eq!(chars.next(), Some(kitty::PLACEHOLDER));
        assert_eq!(chars.next().and_then(kitty::diacritic_index), Some(2), "row");
        assert_eq!(chars.next().and_then(kitty::diacritic_index), Some(3), "column");
        assert_eq!(cell.fg, Color::Rgb(0x01, 0x02, 0x03));
        assert_eq!(buf[(0, 0)].symbol(), " ", "outside the area is untouched");

        let sent = state.take_pending().expect("image to send");
        let cmds = kittychart_testkit::kitty::decode_transmissions(&sent).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].keys.get("U").map(String::as_str), Some("1"), "virtual placement");
        assert_eq!((cmds[0].width, cmds[0].height), (160, 96), "exact cell pixels");

        // Same chart and size: nothing new to send.
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert!(state.take_pending().is_none());
        // Deleting frees the image and forces a re-send next time.
        let del = state.delete_image(false);
        assert!(del.starts_with(b"\x1b_Ga=d,d=I,i=66051"), "{}", String::from_utf8_lossy(&del));
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert!(state.take_pending().is_some());
        // Resized: re-sent.
        ChartWidget::new(&cfg, g).render(Rect::new(2, 1, 24, 6), &mut buf, &mut state);
        assert!(state.take_pending().is_some());
    }

    /// A changed frame goes out under the other id while the placeholders keep the old one
    /// (re-sending under the same id blanks the image until the transfer ends); the next draw
    /// switches the placeholders and frees the old image.
    #[test]
    fn changed_frames_are_double_buffered() {
        let cfg = config();
        let area = Rect::new(0, 0, 20, 6);
        let mut buf = Buffer::empty(area);
        let mut state = ChartState::new(5);
        let g = Graphics::Kitty { cell_width: 8.0, cell_height: 16.0 };
        let id_of = |buf: &Buffer| {
            kitty::placeholder_fg(5) == {
                let Color::Rgb(r, g, b) = buf[(3, 3)].fg else { panic!() };
                (r, g, b)
            }
        };
        let third_diacritic = |buf: &Buffer| buf[(3, 3)].symbol().chars().nth(3);
        let ids = |bytes: &[u8]| -> Vec<String> {
            let s = String::from_utf8_lossy(bytes);
            s.split("\x1b_G")
                .filter_map(|c| {
                    let keys = c.split(';').next()?;
                    let a = keys.split(',').find(|k| k.starts_with("a="))?;
                    let i = keys.split(',').find(|k| k.starts_with("i="))?;
                    Some(format!("{a}:{i}"))
                })
                .collect()
        };
        let alt = kitty::alternate_id(5);

        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert_eq!(
            ids(&state.take_pending().unwrap()),
            ["a=T:i=5"],
            "first frame: straight to the placeholders"
        );
        assert!(!state.needs_redraw());
        assert!(id_of(&buf) && third_diacritic(&buf).is_none());

        // Hover: the new frame goes to the other id; the placeholders still show id 5.
        assert!(state.handle_mouse(10, 3));
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert_eq!(ids(&state.take_pending().unwrap()), [format!("a=T:i={alt}")]);
        assert!(third_diacritic(&buf).is_none(), "still the old image");
        assert!(state.needs_redraw());

        // The next draw switches to it and frees the old one, sending nothing new.
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert_eq!(third_diacritic(&buf).and_then(kitty::diacritic_index), Some(1), "id {alt}");
        assert_eq!(ids(&state.take_pending().unwrap()), ["a=d:i=5"]);
        assert!(!state.needs_redraw());

        // And back: the next change goes to id 5 again.
        assert!(state.clear_hover());
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert_eq!(ids(&state.take_pending().unwrap()), ["a=T:i=5"]);
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert_eq!(ids(&state.take_pending().unwrap()), [format!("a=d:i={alt}")]);
        assert!(third_diacritic(&buf).is_none());

        // Nothing changed: nothing to send or redraw.
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert!(state.take_pending().is_none() && !state.needs_redraw());
    }

    #[test]
    fn hover_redraws_with_a_tooltip() {
        let cfg = ChartConfig::from_json(
            r#"{"type":"bar","data":{"labels":["a","b","c"],"datasets":[{"label":"Sales","data":[1,3,2]}]},
                "options":{"interaction":{"mode":"index","intersect":false}}}"#,
        )
        .unwrap();
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let mut state = ChartState::new(1);
        let g = Graphics::Cells(CellMode::Auto);
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        assert!(!state.handle_mouse(50, 50), "outside the widget");
        assert!(state.handle_mouse(20, 6), "inside: hover changes");
        ChartWidget::new(&cfg, g).render(area, &mut buf, &mut state);
        let text: String = (0..12)
            .map(|y| (0..40).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>() + "\n")
            .collect();
        assert!(text.contains("Sales: 3"), "tooltip:\n{text}");
        assert!(state.clear_hover());
    }

    #[test]
    fn cells_draw_text_and_blocks() {
        let cfg = config();
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 12));
        let mut state = ChartState::new(1);
        ChartWidget::new(&cfg, Graphics::Cells(CellMode::Auto)).render(buf.area, &mut buf, &mut state);
        let text: String = (0..12)
            .map(|y| (0..40).map(|x| buf[(x, y)].symbol().to_owned()).collect::<String>() + "\n")
            .collect();
        assert!(text.contains('█'), "{text}");
        assert!(text.contains(" a ") && text.contains(" b "), "{text}");
        assert!(state.take_pending().is_none());
    }
}
