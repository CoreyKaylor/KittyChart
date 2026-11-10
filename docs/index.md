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

- **Scriptable options:** wherever Chart.js accepts a function, put the marker
  `{"__kc_fn": "name"}` and register the function. In Rust that's `ChartConfig::with_fn("name", |ctx| …)`.
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
kittychart chart.json --out chart.png    # or render a PNG
cat chart.json | kittychart --protocol text --cols 80
```

## Python

```sh
pip install kittychart
```

```python
import kittychart
chart = kittychart.Chart(type="bar", data={"labels": ["Q1", "Q2"], "datasets": [{"label": "Revenue", "data": [12, 19]}]})
chart.show()
```

numpy, pandas and polars data, Rich layouts, a Textual widget, terminal IPython and Jupyter.
See [the Python README](../bindings/python/README.md).

## Node.js, Bun, Deno

```sh
npm install kittychart
```

```js
const { Chart } = require('kittychart')
new Chart({ type: 'line', data: { labels: ['a', 'b'], datasets: [{ label: 'x', data: [1, 3] }] } }).show()
```

Typed arrays, Dates, and typings that accept Chart.js's own `ChartConfiguration`. See
[the Node README](../bindings/node/README.md).

## Rust

```rust
let config = kittychart::ChartConfig::from_json(json)?;
kittychart::show::show(config, &Default::default())?;              // draw in the terminal
let png = kittychart::render_json(json, &kittychart::RenderOptions::new(800.0))?.to_png();
```

`kittychart-ratatui` is a ratatui widget (kitty graphics through Unicode placeholders, or text).

## C and other languages

A C ABI with a generated header: see [the C guide](c.md). It works from any language with a C
FFI (Go, Zig, Ruby, Lua, Swift …).

## How it chooses what to draw

`protocol: auto` (the default, or `$KITTYCHART_PROTOCOL`) asks the terminal what it supports.
It uses kitty graphics if available, then sixel, then text. Inside tmux, kitty images go
through passthrough as Unicode placeholders (needs `set -g allow-passthrough on`). Without a
terminal (a pipe, CI), charts are drawn as text on stdout. Terminal detection currently needs
Unix. On Windows, `show` draws text, and PNG output works everywhere.
