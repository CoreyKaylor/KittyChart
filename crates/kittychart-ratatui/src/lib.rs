//! A [ratatui](https://ratatui.rs) widget that draws Chart.js charts with KittyChart.
//!
//! With kitty graphics, the chart is a real image shown through **Unicode placeholders**: the
//! widget fills its area with placeholder cells, and the image travels separately. It therefore
//! moves, scrolls and clips with the text grid like any other widget, and works inside tmux.
//! Without graphics, the chart is drawn with block and Braille characters.
//!
//! The widget never writes to the terminal itself (ratatui widgets only fill a buffer). After
//! drawing, send the image with [`ChartState::take_pending`]:
//!
//! ```no_run
//! # use kittychart_ratatui::{ChartState, ChartWidget, Graphics};
//! # fn demo(terminal: &mut ratatui::DefaultTerminal, config: &kittychart::ChartConfig) -> std::io::Result<()> {
//! let mut state = ChartState::new(1);
//! let graphics = Graphics::Kitty { cell_width: 10.0, cell_height: 20.0 };
//! terminal.draw(|f| {
//!     f.render_stateful_widget(ChartWidget::new(config, graphics), f.area(), &mut state)
//! })?;
//! if let Some(bytes) = state.take_pending() {
//!     use std::io::Write;
//!     std::io::stdout().write_all(&bytes)?;
//!     std::io::stdout().flush()?;
//! }
//! # Ok(()) }
//! ```
//!
//! The image is only re-sent when the chart, its size or the colors change.

use std::hash::{DefaultHasher, Hash, Hasher};

use kittychart::terminal::{TermGeometry, TermOptions, render_for_terminal};
use kittychart::{ChartConfig, TerminalColors};
use kittychart_canvas::cells::CellMode;
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

/// Per-widget state: the kitty image id, and what has been sent.
#[derive(Debug, Clone)]
pub struct ChartState {
    image_id: u32,
    sent: Option<u64>,
    pending: Option<Vec<u8>>,
}

impl ChartState {
    /// `image_id` must be unique among the images your app shows (1..=0xFFFFFF).
    pub fn new(image_id: u32) -> Self {
        Self { image_id: image_id.clamp(1, 0x00FF_FFFF), sent: None, pending: None }
    }

    pub fn image_id(&self) -> u32 {
        self.image_id
    }

    /// Escape sequences to write to the terminal after drawing (the image, when it changed).
    pub fn take_pending(&mut self) -> Option<Vec<u8>> {
        self.pending.take()
    }

    /// Escapes that free this widget's image in the terminal (write them on exit; the image
    /// otherwise stays in the terminal's memory after the placeholders are gone).
    pub fn delete_image(&mut self, tmux: bool) -> Vec<u8> {
        self.sent = None;
        let mut out = Vec::new();
        kitty::delete_image(self.image_id, kitty::Quiet::Silent, &mut out);
        if tmux { kitty::tmux_passthrough(&out) } else { out }
    }

    /// Forget what was sent, e.g. after the terminal was cleared or images deleted.
    pub fn invalidate(&mut self) {
        self.sent = None;
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
                let key = self.key(area);
                if state.sent != Some(key) {
                    let geom = TermGeometry { cols: area.width, cell_width, cell_height };
                    let opts = TermOptions {
                        cols: Some(area.width),
                        rows: Some(area.height),
                        colors: self.colors.clone(),
                        ..TermOptions::default()
                    };
                    let image = render_for_terminal(self.config.clone(), geom, &opts).image;
                    let mut t = TransmitOptions::new(state.image_id);
                    t.columns = Some(u32::from(area.width));
                    t.rows = Some(u32::from(area.height));
                    let mut esc = Vec::new();
                    kitty::transmit_virtual(image.as_term_image(), &t, &mut esc);
                    state.pending = Some(if self.tmux { kitty::tmux_passthrough(&esc) } else { esc });
                    state.sent = Some(key);
                }
                let (r, g, b) = kitty::placeholder_fg(state.image_id);
                let style = Style::new().fg(Color::Rgb(r, g, b));
                for row in 0..area.height {
                    for col in 0..area.width {
                        if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                            cell.set_symbol(&kitty::placeholder_cell(
                                u32::from(row),
                                u32::from(col),
                                state.image_id,
                            ))
                            .set_style(style);
                        }
                    }
                }
            }
            Graphics::Cells(mode) => {
                let grid = kittychart::render_cells(
                    self.config.clone(),
                    u32::from(area.width),
                    u32::from(area.height),
                    mode,
                    self.colors.as_ref().map(kittychart::Theme::from_terminal),
                );
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
