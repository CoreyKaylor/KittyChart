//! Two charts in a ratatui layout: `cargo run -p kittychart-ratatui --example dashboard`.
//! Uses kitty graphics when the terminal supports them, text cells otherwise. Hover for
//! tooltips, click a legend item to hide that series; `space` animates to new data, `q` quits.

use std::io::Write;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, MouseButton, MouseEventKind};
use kittychart::{ChartConfig, TerminalColors};
use kittychart_canvas::cells::CellMode;
use kittychart_ratatui::{ChartState, ChartWidget, Graphics};
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Paragraph};

fn bar(seed: u64) -> ChartConfig {
    let v: Vec<u64> = (0..6).map(|i| (seed * 7 + i * 13) % 20 + 2).collect();
    ChartConfig::from_json(&format!(
        r#"{{"type":"bar","data":{{"labels":["Mon","Tue","Wed","Thu","Fri","Sat"],
            "datasets":[{{"label":"Deploys","data":{v:?}}}]}},
            "options":{{"maintainAspectRatio":false,"plugins":{{"legend":{{"display":false}}}}}}}}"#
    ))
    .unwrap()
}

fn line(seed: u64) -> ChartConfig {
    let a: Vec<f64> = (0..24).map(|i| ((i as f64 + seed as f64) / 3.0).sin() * 40.0 + 50.0).collect();
    let b: Vec<f64> = (0..24).map(|i| ((i as f64 + seed as f64) / 5.0).cos() * 25.0 + 40.0).collect();
    let labels: Vec<String> = (0..24).map(|h| format!("{h:02}:00")).collect();
    ChartConfig::from_json(&format!(
        r#"{{"type":"line","data":{{"labels":{labels:?},"datasets":[
            {{"label":"p50 ms","data":{a:?},"tension":0.3,"pointRadius":0}},
            {{"label":"p99 ms","data":{b:?},"tension":0.3,"pointRadius":0}}]}},
            "options":{{"maintainAspectRatio":false,
                "interaction":{{"mode":"index","intersect":false}}}}}}"#
    ))
    .unwrap()
}

fn main() -> std::io::Result<()> {
    // Ask the terminal what it can do before ratatui takes over the screen.
    let tty = kittychart_term::tty::Tty::open()?;
    let caps = tty.query(Duration::from_millis(250))?;
    let ws = tty.window_size()?;
    let tmux = kittychart_term::tty::in_tmux();
    let graphics = match ws.cell_px().or(caps.cell_px) {
        Some((w, h)) if caps.kitty_graphics => {
            Graphics::Kitty { cell_width: f32::from(w), cell_height: f32::from(h) }
        }
        _ => Graphics::Cells(CellMode::Auto),
    };
    let colors = caps.background.zip(caps.foreground).map(|(background, foreground)| TerminalColors {
        background,
        foreground,
        palette: caps.full_palette(),
    });

    let mut terminal = ratatui::init();
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableFocusChange
    )?;
    let (mut s1, mut s2) = (ChartState::new(1), ChartState::new(2));
    let mut seed = 1;
    let start = std::time::Instant::now();
    let result = loop {
        let now = start.elapsed().as_secs_f64() * 1000.0;
        s1.set_time(now);
        s2.set_time(now);
        let (bar, line) = (bar(seed), line(seed));
        let drawn = terminal.draw(|f| {
            let [top, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(f.area());
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(40), Constraint::Fill(1)]).areas(body);
            f.render_widget(
                Paragraph::new(format!(
                    "KittyChart + ratatui ({graphics:?})  hover · click legend · space new data · q quit"
                )),
                top,
            );
            for (area, cfg, state, title) in
                [(left, &bar, &mut s1, " Deploys "), (right, &line, &mut s2, " Latency ")]
            {
                let block = Block::bordered().title(title);
                let inner = block.inner(area);
                f.render_widget(block, area);
                f.render_stateful_widget(
                    ChartWidget::new(cfg, graphics).colors(colors.clone()).tmux(tmux),
                    inner,
                    state,
                );
            }
        });
        if let Err(e) = drawn {
            break Err(e);
        }
        // Images go out after the frame; placeholders already reference them by id.
        let mut out = std::io::stdout();
        for s in [&mut s1, &mut s2] {
            if let Some(bytes) = s.take_pending() {
                out.write_all(&bytes)?;
            }
        }
        out.flush()?;
        // A new frame went out under the other image id: draw again to switch to it.
        if s1.needs_redraw() || s2.needs_redraw() {
            continue;
        }
        // While a chart animates, draw again soon; otherwise wait for input.
        let animating = s1.is_animating() || s2.is_animating();
        if !event::poll(Duration::from_millis(if animating { 16 } else { 1000 }))? {
            continue;
        }
        match event::read()? {
            Event::Key(k) if k.code == KeyCode::Char('q') => break Ok(()),
            Event::Key(k) if k.code == KeyCode::Char(' ') => seed += 1,
            // The pointer left the window (with focus-follows-mouse) or the window lost focus:
            // terminals report no mouse-leave, so drop the hover here.
            Event::FocusLost => {
                s1.clear_hover();
                s2.clear_hover();
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Moved => {
                    for s in [&mut s1, &mut s2] {
                        if !s.handle_mouse(m.column, m.row) {
                            s.clear_hover();
                        }
                    }
                }
                MouseEventKind::Down(MouseButton::Left) => {
                    for s in [&mut s1, &mut s2] {
                        s.handle_click(m.column, m.row);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    };
    crossterm::execute!(
        std::io::stdout(),
        crossterm::event::DisableFocusChange,
        crossterm::event::DisableMouseCapture
    )?;
    ratatui::restore();
    if matches!(graphics, Graphics::Kitty { .. }) {
        let mut out = std::io::stdout();
        out.write_all(&[s1.delete_image(tmux), s2.delete_image(tmux)].concat())?;
        out.flush()?;
    }
    result
}
