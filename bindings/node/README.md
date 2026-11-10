# kittychart (Node.js, Bun, Deno)

Chart.js-compatible charts for graphics-capable terminals. Draws with the kitty graphics
protocol (kitty, Ghostty, WezTerm, Konsole …), sixel (foot, xterm, mlterm, Windows Terminal …)
or block and Braille characters anywhere else. The renderer is native (Rust, via Node-API);
no browser or canvas package.

```js
const { Chart } = require('kittychart') // or: import { Chart } from 'kittychart'

const chart = new Chart({
  type: 'bar',
  data: {
    labels: ['Q1', 'Q2', 'Q3', 'Q4'],
    datasets: [{ label: 'Revenue', data: [12, 19, 7, 15] }],
  },
  options: { plugins: { title: { display: true, text: 'Quarterly revenue' } } },
})

chart.show()                                    // draw at the cursor
fs.writeFileSync('chart.png', chart.toPng({ width: 800, scale: 2 }))
console.log(chart.toText({ cols: 60 }))         // block/Braille text with ANSI colors
fs.writeFileSync('chart.svg', chart.toSvg())    // SVG, text as outlines
```

Any Chart.js configuration works, and the typings accept Chart.js's own `ChartConfiguration`
types, so existing configs carry over. Charts take their default colors from your terminal
theme (`colors: 'chartjs'` to opt out).

- **Data**: typed arrays (`Float64Array` …), `BigInt`s, `Date`s (time scales) and `NaN`
  (a gap) can go straight into `data`.
- **`show(opts)`**: `protocol` ('auto', 'kitty', 'sixel', 'text', 'blocks', 'half',
  'braille'; default `$KITTYCHART_PROTOCOL` or 'auto'), `cols`/`rows` (cells),
  `width`/`height` (CSS px), `scale`, `background`, `placeholders` (kitty Unicode placeholders;
  automatic inside tmux), `force` (skip detection), `colors`.
- **TUI frameworks**: `probe()` once, then `chart.prepare(info, { placeholders: true })` returns
  the escapes (`data.subarray(0, graphicsLen)`) and placeholder text to place in your layout.
- `update({ options, data })` merges changes; `toRgba()` returns raw pixels.

Works in Node.js ≥ 18, Bun and Deno (`deno run -A`). Terminal detection needs a Unix terminal.
On Windows, `show()` draws text, and PNG/RGBA rendering works everywhere.
