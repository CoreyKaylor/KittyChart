"""KittyChart: Chart.js-compatible charts for graphics-capable terminals.

>>> import kittychart
>>> chart = kittychart.Chart(type="bar", data={"labels": ["Q1", "Q2"], "datasets": [{"label": "Revenue", "data": [12, 19]}]})
>>> chart.show()                 # kitty graphics, sixel or text, whatever the terminal supports
>>> png = chart.to_png(width=800)

``rich.print(chart)`` draws inline with Rich, ``kittychart.textual.ChartWidget`` is a Textual
widget, and in IPython displaying a chart draws it (Jupyter shows the PNG).
"""

from ._chart import Chart, probe, show, terminal_info
from ._native import __version__

__all__ = ["Chart", "show", "probe", "terminal_info", "__version__"]
