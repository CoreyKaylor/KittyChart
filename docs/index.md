# KittyChart

Chart.js-compatible charts for terminals that can show images: kitty, Ghostty, WezTerm,
Konsole and others through the kitty graphics protocol; foot, xterm, mlterm and Windows
Terminal through sixel; and every other terminal as block and Braille characters.

You write a normal Chart.js configuration (`{ type, data, options }`). KittyChart renders it
natively in Rust, with Chart.js's defaults, layout and palette. It sizes the chart to your
terminal's cells and draws it at the cursor, with default colors taken from your terminal
theme.

**[Gallery](gallery.md)**: every test fixture, on a white page and in a dark terminal.

## Beyond plain JSON

- **Scriptable options:** wherever Chart.js accepts a function, pass one. In Python and Node,
  put the function in the config itself. In JSON, put the marker `{"__kc_fn": "name"}` and
  register it: `ChartConfig::with_fn("name", |ctx| …)` in Rust, `kc_chart_set_dispatch` in
  [C](c.md).
  The function gets Chart.js's context object (`type`, `datasetIndex`, `dataIndex`, `raw`,
  `parsed`, `chart.chartArea` …). `ticks.callback` gets `(value, index, ticks)` as `__args`.
- **Gradients** work in plain JSON wherever a color goes:
  `{"type": "linear", "x0": 0, "y0": "bottom", "x1": 0, "y1": "top", "stops": [[0, "#36a2eb00"], [1, "#36a2eb"]]}`.
  Radial and conic work too, and coordinates may be chart-area keywords (`left`, `top`, `centerX` …).
- **Line segment styles:** `segment: { borderColor, borderDash, borderWidth, backgroundColor … }`,
  static or scriptable, with a `segment` context (`p0`, `p1`, `p0DataIndex` …).
- **SVG:** `kittychart chart.json --out chart.svg`, `render_svg()`, `to_svg()`, `toSvg()` or
  `kc_render_svg()`. Text is drawn as outlines, so the file needs no fonts.

## Command line

```sh
kittychart chart.json                    # draw in the terminal (kitty → sixel → text)
kittychart chart.json --out chart.png    # or render a PNG (or .svg)
cat chart.json | kittychart --protocol text --cols 80
kittychart chart.json --watch            # redraw in place whenever the file changes
kittychart view chart.json --watch       # explore it full-screen (below)
```

## Interactive charts

`kittychart view chart.json` opens the chart full-screen. Hover shows Chart.js's tooltip and
hover styles, following the config's `interaction` and `hover` options. Clicking a legend item
hides that dataset (or that slice, on pie, doughnut and polar charts). With `--watch`, the
chart animates to the file's new contents whenever it changes.

| Key | |
|---|---|
| mouse | hover for tooltips; click legend items |
| `←` `→` `Home` `End` | step through the data points |
| `1`–`9` | toggle a dataset (a slice on arc charts) |
| `l` | show or hide the legend |
| `r` | replay the animation |
| `s` | save a PNG to the current directory |
| `q`, `Esc` | quit, restoring the screen |

It works with kitty graphics, sixel and text. When the terminal says it can read local files
or shared memory (kitty does), frames go that way instead of through the escape stream;
otherwise they are sent inline (`--no-fast-transfer` forces that).
Python and Node have the same viewer as `chart.interact()`, and the ratatui and Textual widgets
handle hover, legend clicks and animation inside your own layout. Animations (Chart.js's
`animation` options) play only in these interactive views. A chart printed with `show` draws
its final frame at once.

## Python

```sh
pip install kittychart
```

```python
import kittychart
chart = kittychart.Chart(type="bar", data={"labels": ["Q1", "Q2"], "datasets": [{"label": "Revenue", "data": [12, 19]}]})
chart.show()
```

numpy, pandas and polars data, Python functions as scriptable options, Rich layouts, a Textual
widget with hover, terminal IPython and Jupyter, and `chart.interact()`. See
[the Python README](python.md).

## Node.js, Bun, Deno

```sh
npm install kittychart
```

```js
const { Chart } = require('kittychart')
new Chart({ type: 'line', data: { labels: ['a', 'b'], datasets: [{ label: 'x', data: [1, 3] }] } }).show()
```

Typed arrays, Dates, JS functions as scriptable options, `chart.interact()`, and typings that
accept Chart.js's own `ChartConfiguration`. Prebuilt for glibc and musl (Alpine) Linux, macOS
and Windows. See [the Node README](node.md).

## Rust

```rust
let config = kittychart::ChartConfig::from_json(json)?;
kittychart::show::show(config, &Default::default())?;              // draw in the terminal
let png = kittychart::render_json(json, &kittychart::RenderOptions::new(800.0))?.to_png();
```

`kittychart-ratatui` is a ratatui widget (kitty graphics through Unicode placeholders, or text).
Its `ChartState` handles hover (`handle_mouse`), legend clicks (`handle_click`) and animation
(`set_time`, `is_animating`); see `examples/dashboard.rs`. `kittychart::interact` is the
full-screen viewer, and `kittychart::session::Session` is its sans-IO core for other hosts.

## C and other languages

A C ABI with a generated header: see [the C guide](c.md). It works from any language with a C
FFI (Go, Zig, Ruby, Lua, Swift …).

## How it chooses what to draw

`protocol: auto` (the default, or `$KITTYCHART_PROTOCOL`) asks the terminal what it supports.
It uses kitty graphics if available, then sixel, then text. Inside tmux, kitty images go
through passthrough as Unicode placeholders (needs `set -g allow-passthrough on`). Without a
terminal (a pipe, CI), charts are drawn as text on stdout. Detection works on Unix and, using
the console's virtual-terminal mode, on Windows (sixel in Windows Terminal, kitty graphics in
WezTerm). The Windows path cross-compiles and passes clippy, but hasn't been run in a real
Windows terminal yet. PNG and SVG output work everywhere.
