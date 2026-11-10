"""Charts inside Rich layouts: a panel and a table.

    uv run python examples/rich_report.py
"""

from rich.console import Console
from rich.panel import Panel
from rich.table import Table

import kittychart

console = Console()
sales = kittychart.Chart(
    type="bar",
    data={"labels": ["Q1", "Q2", "Q3", "Q4"], "datasets": [{"label": "Revenue", "data": [12, 19, 7, 15]}]},
)
mix = kittychart.Chart(
    type="doughnut",
    data={"labels": ["Web", "iOS", "Android"], "datasets": [{"data": [55, 30, 15]}]},
)

console.print(Panel(sales, title="Quarterly revenue", width=60))
table = Table(title="Platform mix")
table.add_column("Chart", width=40)
table.add_column("Notes")
table.add_row(mix, "Web still leads; Android flat.")
console.print(table)
