"""A Textual dashboard with two charts: hover for tooltips, click a legend item to hide a
series, space animates to new data, q quits.

    uv run python examples/textual_dashboard.py
"""

import os
import random

from textual.app import App, ComposeResult
from textual.containers import Horizontal
from textual.widgets import Footer, Header

import kittychart
from kittychart.textual import ChartWidget  # import before app.run(): probes the terminal

# A fixed seed gives the same data every run (the tests use it); unset, it's random.
random.seed(os.environ.get("KITTYCHART_EXAMPLE_SEED"))

DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]


def bars() -> kittychart.Chart:
    return kittychart.Chart(
        type="bar",
        data={
            "labels": DAYS,
            "datasets": [
                {"label": "Signups", "data": [random.randint(5, 40) for _ in DAYS]},
                {"label": "Churn", "data": [random.randint(1, 15) for _ in DAYS]},
            ],
        },
        options={"maintainAspectRatio": False},
    )


def line() -> kittychart.Chart:
    return kittychart.Chart(
        type="line",
        data={
            "labels": DAYS,
            "datasets": [{"label": "Latency (ms)", "data": [random.uniform(80, 240) for _ in DAYS], "fill": True}],
        },
        options={"maintainAspectRatio": False, "interaction": {"mode": "index", "intersect": False}},
    )


class Dashboard(App):
    BINDINGS = [("space", "refresh_data", "New data"), ("q", "quit", "Quit")]

    def compose(self) -> ComposeResult:
        yield Header()
        with Horizontal():
            yield ChartWidget(bars(), id="bars")
            yield ChartWidget(line(), id="line")
        yield Footer()

    def action_refresh_data(self) -> None:
        self.query_one("#bars", ChartWidget).chart = bars()
        self.query_one("#line", ChartWidget).chart = line()


if __name__ == "__main__":
    Dashboard().run()
