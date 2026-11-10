"""Rich integration: ``rich.print(chart)`` draws the chart inline.

In a terminal with kitty graphics, the image is sent as a zero-width control segment and shown
through Unicode placeholder cells, so it takes part in Rich's layout (panels, tables, columns)
like text. With sixel, the image is written at the cursor. Otherwise, and when the console isn't
a terminal (recording, exporting, Jupyter), the chart is drawn with block and Braille characters.
"""

from __future__ import annotations

import os
from typing import Any, Iterator

from rich.color import Color
from rich.segment import ControlType
from rich.segment import Segment
from rich.style import Style
from rich.text import Text

from . import _native
from ._chart import _show_options, terminal_info

# Rich only treats a segment as a zero-width control (never cropped, padded or measured) when
# its control list is non-empty. The codes only label it; Rich writes the segment's text.
_CONTROL = [(ControlType.BELL,)]


def render(chart: Any, console: Any, options: Any) -> Iterator[Segment]:
    cols = max(10, options.max_width)
    text_mode = "text"
    rows = options.height
    info = None
    if console.is_terminal and not console.is_jupyter and not console.is_dumb_terminal:
        info = terminal_info()
    mode = "text"
    if info is not None:
        mode = "kitty" if info["kitty"] else "sixel" if info["sixel"] else "text"
    # $KITTYCHART_PROTOCOL forces a mode, as it does for show() (graphics only on a terminal).
    forced = os.environ.get("KITTYCHART_PROTOCOL", "")
    if forced in ("text", "blocks", "half", "braille"):
        mode, text_mode = "text", forced
    elif forced in ("kitty", "sixel") and info is not None:
        mode = forced

    if mode == "kitty":
        p = _native.prepare_for(
            chart._json,
            info,
            _show_options("kitty", cols=cols, rows=rows, placeholders=True, image_id=chart._inline_image_id()),
            chart._dispatch,
        )
        yield Segment(p.data[: p.graphics_len].decode("ascii"), None, _CONTROL)
        style = Style(color=Color.from_rgb(*_native.placeholder_color(p.image_id)))
        for i, line in enumerate(_native.placeholder_rows(p.image_id, p.cols, p.rows)):
            if i:
                yield Segment.line()
            yield Segment(line, style)
        return

    if mode == "sixel":
        # A sixel image draws at the cursor and moves it, which Rich can't lay out. So reserve
        # the chart's cells as blank lines (Rich lays those out like any text, scrolling as
        # needed), then at the end of the last line jump back to the block's top-left, draw the
        # image over the blanks and restore the cursor. Text written afterwards never covers it.
        p = _native.prepare_for(chart._json, info, _show_options("sixel", cols=cols, rows=rows), chart._dispatch)
        blank = Segment(" " * p.cols)
        for i in range(p.rows):
            if i:
                yield Segment.line()
            yield blank
        up = f"\x1b[{p.rows - 1}A" if p.rows > 1 else ""
        image = p.data.decode("ascii").rstrip("\n")
        yield Segment(f"\x1b7{up}\x1b[{p.cols}D{image}\x1b8", None, _CONTROL)
        return

    colors = info["colors"] if info else None
    ansi = _native.render_text(chart._json, cols, rows, text_mode, colors, chart._dispatch)
    text = Text.from_ansi(ansi.rstrip("\r\n").replace("\r\n", "\n"), no_wrap=True, overflow="crop")
    yield from text.__rich_console__(console, options)
