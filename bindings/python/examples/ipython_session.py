"""Used by the IPython end-to-end test: `ipython -i examples/ipython_session.py`, then type `c`."""

import kittychart

c = kittychart.Chart(
    type="bar",
    data={"labels": ["Q1", "Q2", "Q3"], "datasets": [{"label": "Revenue", "data": [12, 19, 7]}]},
    options={"aspectRatio": 3},
)
