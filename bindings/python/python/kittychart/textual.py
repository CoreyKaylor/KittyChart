"""A Textual widget that draws a :class:`kittychart.Chart`.

With kitty graphics, the chart is a real image shown through Unicode placeholder cells, so it
scrolls, clips and moves with the layout like any widget (also inside tmux). Otherwise it's
drawn with block and Braille characters. Chart colors follow the widget's Textual theme colors.
Hover shows tooltips, clicking a legend item hides that dataset, and new data animates.

Terminal detection reads the terminal's replies, which can't happen while Textual owns the
input, so this module probes the terminal when it's imported (import it before ``app.run()``,
as usual). If the app is already running, kitty support is guessed from the environment.

>>> from kittychart.textual import ChartWidget
>>> class Dashboard(App):
...     def compose(self):
...         yield ChartWidget(Chart(type="bar", data=...))
"""

from __future__ import annotations

import os
import sys
import time
from typing import Any, Optional

from rich.color import Color as RichColor
from rich.segment import Segment
from rich.style import Style
from rich.text import Text
from textual.reactive import reactive
from textual.strip import Strip
from textual.widget import Widget

from . import _chart, _native
from ._chart import Chart

__all__ = ["ChartWidget"]


def _app_running() -> bool:
    try:
        from textual._context import active_app

        active_app.get()
        return True
    except (ImportError, LookupError):
        return False


if not _chart._PROBED and not _app_running() and sys.stdin.isatty() and sys.stdout.isatty():
    _chart.probe()


def _guess_info() -> Optional[dict]:
    """Terminal info without queries: the kernel's window size, and kitty support guessed from
    the environment."""
    try:
        cols, rows, cw, ch = _native.window_size()
    except OSError:
        return None
    term = os.environ.get("TERM", "")
    program = os.environ.get("TERM_PROGRAM", "")
    kitty = (
        term in ("xterm-kitty", "xterm-ghostty")
        or program in ("ghostty", "WezTerm")
        or "KITTY_WINDOW_ID" in os.environ
        or "GHOSTTY_RESOURCES_DIR" in os.environ
    )
    return {
        "cols": cols,
        "rows": rows,
        "cell_width": float(cw or 10),
        "cell_height": float(ch or 20),
        "kitty": kitty,
        "sixel": False,
        "tmux": bool(os.environ.get("TMUX")),
        "colors": None,
    }


def _info() -> Optional[dict]:
    if _chart._PROBED:
        return _chart._INFO
    return _guess_info()


class ChartWidget(Widget):
    """Draws a chart in its area. Assign :attr:`chart` to update it.

    ``protocol``: "auto" (kitty graphics if available, else text), "kitty", or a text mode
    ("text", "blocks", "half", "braille").
    """

    DEFAULT_CSS = """
    ChartWidget {
        width: 1fr;
        height: 1fr;
    }
    """

    chart: reactive[Optional[Chart]] = reactive(None, layout=False)

    def __init__(
        self,
        chart: Optional[Chart] = None,
        *,
        protocol: str = "auto",
        name: Optional[str] = None,
        id: Optional[str] = None,
        classes: Optional[str] = None,
    ) -> None:
        super().__init__(name=name, id=id, classes=classes)
        if protocol not in ("auto", "kitty", "text", "blocks", "half", "braille"):
            raise ValueError(f"unsupported protocol for Textual: {protocol!r}")
        self.protocol = protocol
        self.image_id = _native.next_image_id()
        self._key: Any = None
        self._lines: list[Strip] = []
        self._sent = False
        # The live chart (hover, tooltip, legend toggles, animation) and what it was built for.
        self._kc_live: Any = None
        self._kc_live_key: Any = None
        self._kc_start = time.monotonic()
        self._kc_timer: Any = None
        self.set_reactive(ChartWidget.chart, chart)

    def watch_chart(self, _old: Optional[Chart], _new: Optional[Chart]) -> None:
        if self._kc_live is not None and _new is not None and self._kc_live_key is not None:
            # Same widget, new data: animate to it.
            self._kc_live.update(_new.to_json())
        self._key = None
        self._kc_animate()
        self.refresh()

    def on_resize(self) -> None:
        self._key = None
        self.refresh()

    def on_unmount(self) -> None:
        if self._sent:
            info = _info()
            self._write(_native.delete_image(self.image_id, bool(info and info["tmux"])))

    # Interaction: hover, click, animation.

    def _kc_cell_px(self) -> tuple:
        """(x, y) scale from cells to chart CSS pixels for the current mode."""
        info = _info()
        if self._mode(info) == "kitty" and info is not None:
            cw, ch = info["cell_width"], info["cell_height"]
            scale = min(4.0, max(1.0, ch / 16.0))
            return cw / scale, ch / scale
        return 8.0, 16.0  # text cells

    def on_mouse_move(self, event: Any) -> None:
        if self._kc_live is None:
            return
        sx, sy = self._kc_cell_px()
        if self._kc_live.hover((event.x + 0.5) * sx, (event.y + 0.5) * sy):
            self._key = None
            self._kc_animate()  # the hover transition (tooltip fade, hover colors)
            self.refresh()

    def on_leave(self, _event: Any) -> None:
        if self._kc_live is not None and self._kc_live.hover(None, None):
            self._key = None
            self._kc_animate()
            self.refresh()

    def on_click(self, event: Any) -> None:
        if self._kc_live is None:
            return
        sx, sy = self._kc_cell_px()
        if self._kc_live.click((event.x + 0.5) * sx, (event.y + 0.5) * sy):
            self._key = None
            self._kc_animate()
            self.refresh()

    def _kc_animate(self) -> None:
        """Redraw at ~30 fps while the chart animates."""
        if self._kc_timer is None and self.is_mounted:
            self._kc_timer = self.set_interval(1 / 30, self._kc_tick)

    def _kc_tick(self) -> None:
        self._key = None
        self.refresh()
        if self._kc_live is not None and not self._kc_live.animating and self._kc_timer is not None:
            self._kc_timer.stop()
            self._kc_timer = None

    def on_mount(self) -> None:
        self._kc_animate()

    def _write(self, data: bytes) -> None:
        driver = getattr(self.app, "_driver", None)
        if driver is not None and not driver.is_headless:
            driver.write(data.decode("ascii"))

    def _colors(self) -> Optional[dict]:
        """The chart's surface and ink: the widget's Textual colors. When the theme uses the
        terminal's own colors (ANSI, e.g. "textual-ansi" or a transparent background), the
        probed terminal colors instead (with the terminal's palette)."""
        _, _, background, color = self.colors
        if background.ansi is not None or color.ansi is not None or background.a == 0:
            info = _info()
            return info["colors"] if info else None
        bg = background.rgb
        fg = (background + color).rgb
        return {"background": list(bg), "foreground": list(fg), "palette": None}

    def _mode(self, info: Optional[dict]) -> str:
        if self.protocol != "auto":
            return self.protocol
        return "kitty" if info and info["kitty"] else "text"

    def _build(self) -> None:
        width, height = self.size.width, self.size.height
        info = _info()
        mode = self._mode(info)
        colors = self._colors()
        if self.chart is None or width < 1 or height < 1:
            self._lines = []
            return
        cells = mode != "kitty" or info is None
        live_key = (id(self.chart), cells, str(colors))
        if self._kc_live is None or (self._kc_live_key[1], self._kc_live_key[2]) != (cells, str(colors)):
            self._kc_live = _native.Live(self.chart.to_json(), self.chart._dispatch, cells, colors)
        self._kc_live_key = live_key
        self._kc_live.set_time((time.monotonic() - self._kc_start) * 1000.0)
        key = (width, height, mode)
        if key == self._key and not self._kc_live.animating:
            return
        self._key = key
        if not cells:
            esc = self._kc_live.render_kitty(
                width, height, info["cell_width"], info["cell_height"], self.image_id, bool(info["tmux"])
            )
            self._write(esc)
            self._sent = True
            style = Style(color=RichColor.from_rgb(*_native.placeholder_color(self.image_id)))
            self._lines = [
                Strip([Segment(row, style)], width)
                for row in _native.placeholder_rows(self.image_id, width, height)
            ]
            return
        text_mode = mode if mode != "kitty" else "text"
        ansi = self._kc_live.render_text(width, height, text_mode)
        text = Text.from_ansi(ansi.replace("\r\n", "\n"), no_wrap=True)
        console = self.app.console
        self._lines = [Strip(list(line.render(console)), line.cell_len) for line in text.split("\n")]

    def render_line(self, y: int) -> Strip:
        self._build()
        # render_line strips carry their own styles: apply the widget's (its background
        # especially, which shows through the chart's transparent pixels). Segment styles win,
        # so placeholder cells keep the color that names their image.
        style = self.rich_style
        if y < len(self._lines):
            return self._lines[y].apply_style(style).extend_cell_length(self.size.width, style)
        return Strip.blank(self.size.width, style)
