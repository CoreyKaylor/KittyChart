import asyncio
import json

import pytest
from IPython.lib.pretty import pretty

import kittychart

BAR = {"type": "bar", "data": {"labels": ["Q1", "Q2"], "datasets": [{"label": "Revenue", "data": [12, 19]}]}}


def test_ipython_pretty_draws_the_chart(headless):
    # Terminal IPython prints the text/plain repr; for a chart that's the drawing itself.
    out = headless(
        "import json, sys, kittychart; from IPython.lib.pretty import pretty; "
        f"sys.stdout.write(json.dumps(pretty(kittychart.Chart({BAR!r}))))",
        env={"KITTYCHART_PROTOCOL": "text"},
    )
    out = json.loads(out)
    assert "Revenue" in out and not out.endswith("\n")


def test_textual_widget_renders_text_mode():
    from textual.app import App

    from kittychart.textual import ChartWidget

    class Demo(App):
        def compose(self):
            yield ChartWidget(kittychart.Chart(BAR), protocol="text", id="c")

    async def main():
        app = Demo()
        async with app.run_test(size=(60, 20)) as pilot:
            w = app.query_one("#c", ChartWidget)
            lines = ["".join(seg.text for seg in w.render_line(y)) for y in range(w.size.height)]
            assert any("Revenue" in l for l in lines)
            assert all(len(l) == 60 for l in lines)
            # Strips carry the widget's background (it shows through transparent chart pixels).
            bg = w.rich_style.bgcolor
            assert bg is not None
            assert all(seg.style is not None and seg.style.bgcolor == bg for y in range(3) for seg in w.render_line(y))
            w.chart = kittychart.Chart(type="line", data={"labels": ["a"], "datasets": [{"label": "Swapped", "data": [1]}]})
            await pilot.pause()
            lines = ["".join(seg.text for seg in w.render_line(y)) for y in range(w.size.height)]
            assert any("Swapped" in l for l in lines)

    asyncio.run(main())


def test_textual_rejects_unknown_protocols():
    from kittychart.textual import ChartWidget

    with pytest.raises(ValueError):
        ChartWidget(kittychart.Chart(BAR), protocol="sixel")
