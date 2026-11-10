"""Draw a chart in the terminal: kitty graphics, sixel or text, whichever it supports.

    uv run python examples/show.py
"""

import numpy as np

import kittychart

x = np.linspace(0, 4 * np.pi, 200)
chart = kittychart.Chart(
    type="line",
    data={
        "labels": [f"{v:.1f}" for v in x],
        "datasets": [
            {"label": "sin", "data": np.sin(x), "pointRadius": 0},
            {"label": "damped", "data": np.sin(x) * np.exp(-x / 6), "pointRadius": 0},
        ],
    },
    options={"plugins": {"title": {"display": True, "text": "numpy → Chart.js → terminal"}}},
)
chart.show(cols=70)
