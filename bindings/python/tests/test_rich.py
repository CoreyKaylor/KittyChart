import io
import re

import pytest
from rich.console import Console
from rich.panel import Panel

import kittychart
import kittychart._chart as c
from kittychart import _native

BAR = {"type": "bar", "data": {"labels": ["Q1", "Q2"], "datasets": [{"label": "Revenue", "data": [12, 19]}]}}
KITTY = {"cols": 80, "rows": 24, "cell_width": 10.0, "cell_height": 20.0, "kitty": True, "sixel": False, "tmux": False, "colors": None}


def render(renderable, terminal=True, width=40):
    f = io.StringIO()
    Console(file=f, force_terminal=terminal, width=width, color_system="truecolor").print(renderable)
    return f.getvalue()


def test_text_when_not_a_terminal():
    out = render(kittychart.Chart(BAR), terminal=False)
    assert "Revenue" in out and "\x1b_G" not in out


def test_kitty_placeholders_survive_rich_layout(monkeypatch):
    monkeypatch.setattr(c, "_INFO", KITTY)
    out = render(Panel(kittychart.Chart(BAR), width=30))
    # The whole escape survives (Rich doesn't crop or pad control segments) …
    escapes = re.findall(r"\x1b_G(.*?)\x1b\\", out, re.S)
    assert escapes and escapes[0].startswith("a=T,U=1,") and escapes[-1].startswith("m=0")
    # … and the placeholder rows fill the panel's inner width: 30 - 2 borders - 2 padding.
    lines = [l for l in out.split("\n") if "\U0010EEEE" in l]
    assert lines and all(l.count("\U0010EEEE") == 26 for l in lines)
    image_id = int(re.search(r",i=(\d+)", escapes[0]).group(1))
    r, g, b = _native.placeholder_color(image_id)
    assert f"38;2;{r};{g};{b}m" in out


def test_sixel_reserves_its_cells_then_draws_over_them(monkeypatch):
    monkeypatch.setattr(c, "_INFO", dict(KITTY, kitty=False, sixel=True))
    out = render(Panel(kittychart.Chart(BAR), width=30))
    lines = out.rstrip("\n").split("\n")
    # Every panel line keeps both borders: the chart's rows are blank cells to Rich.
    body = lines[1:-1]
    assert body and all(l.startswith("│") and l.endswith("│") for l in [re.sub(r"\x1b7.*?\x1b8", "", l) for l in body])
    # The image is drawn from the end of its last row: save, up rows-1, left cols, sixel, restore.
    m = re.search(r"\x1b7(?:\x1b\[(\d+)A)?\x1b\[(\d+)D\x1bP.*?\x1b\\\x1b8", out, re.S)
    assert m, "positioned sixel"
    rows = len(body)  # Panel's default padding is horizontal only
    assert int(m.group(1) or 0) == rows - 1 and int(m.group(2)) == 26
    assert m.group(0) in body[-1] and body[-1].index("\x1b7") < body[-1].rindex("│"), (
        "drawn at the end of the chart's last row, before the right border"
    )


def test_protocol_env_forces_text(monkeypatch):
    monkeypatch.setattr(c, "_INFO", KITTY)
    monkeypatch.setenv("KITTYCHART_PROTOCOL", "braille")
    out = render(kittychart.Chart(BAR))
    assert "\x1b_G" not in out and any("\u2800" <= ch <= "\u28ff" for ch in out)


def test_text_follows_terminal_colors(monkeypatch):
    dark = dict(KITTY, kitty=False, colors={"background": [0, 0, 0], "foreground": [250, 250, 250], "palette": None})
    monkeypatch.setattr(c, "_INFO", dark)
    assert render(kittychart.Chart(BAR)) != render(kittychart.Chart(BAR), terminal=False)


def test_rerenders_reuse_the_image_until_the_config_changes(monkeypatch):
    monkeypatch.setattr(c, "_INFO", KITTY)
    chart = kittychart.Chart(BAR)
    ids = lambda out: set(re.findall(r"\x1b_Ga=T,U=1,.*?,i=(\d+)", out))
    first, second = ids(render(chart)), ids(render(chart))
    assert len(first) == 1 and first == second, "rich.live re-renders replace one image"
    chart.update(options={"plugins": {"title": {"display": True, "text": "new"}}})
    assert ids(render(chart)) != first, "a changed chart is a new image; earlier output keeps its own"
