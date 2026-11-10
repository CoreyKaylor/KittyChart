# kittychart (Python)

Chart.js-compatible charts for graphics-capable terminals. Draws with the kitty graphics
protocol (kitty, Ghostty, WezTerm, Konsole …), sixel (foot, xterm, mlterm, Windows Terminal …)
or block and Braille characters anywhere else. The renderer is Rust; no browser or Node.

```python
import kittychart

chart = kittychart.Chart(
    type="line",
    data={
        "labels": ["Mon", "Tue", "Wed", "Thu", "Fri"],
        "datasets": [{"label": "Requests", "data": [120, 190, 170, 250, 210], "fill": True}],
    },
    options={"plugins": {"title": {"display": True, "text": "This week"}}},
)
chart.show()                       # draw at the cursor
chart.save("chart.png", width=800, scale=2)
print(chart.to_text(cols=60))      # block/Braille text with ANSI colors
```

Any Chart.js configuration works: the same `type`, `data` and `options`, the same defaults.
Charts take their default colors from your terminal theme (`colors="chartjs"` to opt out).

**Data**: numpy arrays, pandas and polars Series, `datetime`s (time scales) and NaN (a gap) can
go straight into `data`. `Chart.from_dataframe(df, x="date", y=["a", "b"])` makes one dataset
per column, for pandas or polars.

**Rich**: `rich.print(chart)`, or put a chart in a `Panel`, `Table` or `Columns`. With kitty
graphics the image lives in Rich's layout through Unicode placeholders.

**Textual**: `from kittychart.textual import ChartWidget`; `ChartWidget(chart)` fills its area,
redraws on resize, and `widget.chart = new_chart` updates it.

**IPython / Jupyter**: in terminal IPython, displaying a chart (`chart` on its own line) draws
it. Jupyter (and euporie) show it as a PNG.

**`show()` options**: `protocol` ("auto", "kitty", "sixel", "text", "blocks", "half",
"braille"; default `$KITTYCHART_PROTOCOL` or "auto"), `cols`/`rows` (cells), `width`/`height`
(CSS px), `scale`, `background`, `placeholders` (kitty Unicode placeholders, automatic inside
tmux), `force` (skip detection), `colors` ("terminal" or "chartjs").
