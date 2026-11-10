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
chart.save("chart.png", width=800, scale=2)   # or "chart.svg"
print(chart.to_text(cols=60))      # block/Braille text with ANSI colors
```

Any Chart.js configuration works: the same `type`, `data` and `options`, the same defaults.
Charts take their default colors from your terminal theme (`colors="chartjs"` to opt out).

**Data**: numpy arrays, pandas and polars Series, `datetime`s (time scales) and NaN (a gap) can
go straight into `data`. `Chart.from_dataframe(df, x="date", y=["a", "b"])` makes one dataset
per column, for pandas or polars.

**Rich**: `rich.print(chart)`, or put a chart in a `Panel`, `Table` or `Columns`. With kitty
graphics the image lives in Rich's layout through Unicode placeholders.

**Functions as options**: wherever Chart.js takes a function, pass a Python callable. It gets
Chart.js's context (`ctx.raw`, `ctx.dataIndex`, `ctx.chart.chartArea` …); tick callbacks get
`(value, index, ticks)`:

```python
kittychart.Chart(
    type="bar",
    data={"labels": ["a", "b", "c"], "datasets": [{"data": [5, 15, 8],
        "backgroundColor": lambda ctx: "#e74c3c" if ctx.raw > 10 else "#3498db"}]},
    options={"scales": {"y": {"ticks": {"callback": lambda value, i, ticks: f"${value}"}}}},
).show()
```

**Interactive**: `chart.interact()` explores the chart full-screen until `q`: hover for
tooltips, click legend items to hide datasets, arrows to step through the data, `r` replays the
animation, `s` saves a PNG. `chart.interact(watch="chart.json")` animates to that file's
contents whenever it changes.

**Textual**: `from kittychart.textual import ChartWidget`; `ChartWidget(chart)` fills its area,
redraws on resize, shows tooltips on hover, toggles datasets on legend clicks, and animates
when `widget.chart = new_chart` updates it.

**IPython / Jupyter**: in terminal IPython, displaying a chart (`chart` on its own line) draws
it. Jupyter (and euporie) show it as a PNG.

**`show()` options**: `protocol` ("auto", "kitty", "sixel", "text", "blocks", "half",
"braille"; default `$KITTYCHART_PROTOCOL` or "auto"), `cols`/`rows` (cells), `width`/`height`
(CSS px), `scale`, `background`, `placeholders` (kitty Unicode placeholders, automatic inside
tmux), `force` (skip detection), `colors` ("terminal" or "chartjs").
