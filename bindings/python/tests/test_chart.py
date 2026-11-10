import datetime as dt
import io
import json
import math

import numpy as np
import pandas as pd
import polars as pl
import pytest

import kittychart
from conftest import GOLDENS, decode_png

BAR = {"type": "bar", "data": {"labels": ["Q1", "Q2", "Q3"], "datasets": [{"label": "Revenue", "data": [12, 19, 7]}]}}


def test_construct_from_dict_or_keywords():
    a = kittychart.Chart(BAR)
    b = kittychart.Chart(type="bar", data=BAR["data"])
    assert a.to_json() == b.to_json()
    assert kittychart.Chart.from_json(json.dumps(BAR)).to_png() == a.to_png()
    assert "type='bar'" in repr(a) and "datasets=1" in repr(a)


def test_invalid_configs_raise_value_error():
    with pytest.raises(ValueError):
        kittychart.Chart({"type": "nope", "data": {}})
    with pytest.raises(ValueError, match="color"):
        kittychart.Chart(BAR).to_png(background="not-a-color")
    with pytest.raises(ValueError, match="width"):
        kittychart.Chart(BAR).to_png(width=0)
    with pytest.raises(TypeError):
        kittychart.Chart(type="bar", data={"datasets": [{"data": [object()]}]})


@pytest.mark.parametrize("name", ["bar/grouped.json", "line/basic.json", "arc/pie.json"])
def test_pixels_match_the_rust_goldens(fixture_config, name):
    cfg, kc = fixture_config(name)
    w, h, rgba = kittychart.Chart(cfg).to_rgba(
        kc.get("width", 512), kc.get("height"), scale=kc.get("dpr", 1), background=kc.get("background", "#ffffff")
    )
    gw, gh, golden = decode_png((GOLDENS / name.replace(".json", ".png")).read_bytes())
    assert (w, h) == (gw, gh)
    # Same renderer, same options: identical on the platform the goldens were made on, and
    # within anti-aliasing noise elsewhere.
    diff = sum(abs(a - b) for a, b in zip(rgba, golden)) / len(rgba)
    assert diff < 0.5


def test_png_and_rgba_sizes():
    c = kittychart.Chart(BAR)
    w, h, rgba = c.to_rgba(400, scale=2)
    assert (w, h) == (800, 400) and len(rgba) == w * h * 4
    png = c.to_png(300, 300, background="white")
    assert png[:4] == b"\x89PNG" and decode_png(png)[:2] == (300, 300)


def test_save(tmp_path):
    p = tmp_path / "c.png"
    kittychart.Chart(BAR).save(p, width=200)
    assert p.read_bytes()[:4] == b"\x89PNG"


def test_numpy_pandas_polars_and_nan_become_chartjs_data():
    arr = np.array([1.5, np.nan, 3.0])
    c = kittychart.Chart(
        type="line",
        data={
            "labels": pd.Index(["a", "b", "c"]),
            "datasets": [
                {"data": arr},
                {"data": pd.Series([1, 2, 3], dtype="int64")},
                {"data": pl.Series([4.0, None, math.inf])},
                {"data": [np.float32(1.25), np.int64(2), None]},
            ],
        },
    )
    d = json.loads(c.to_json())["data"]
    assert d["labels"] == ["a", "b", "c"]
    assert [s["data"] for s in d["datasets"]] == [[1.5, None, 3.0], [1, 2, 3], [4.0, None, None], [1.25, 2, None]]


def test_dates_become_iso_strings_for_time_scales():
    days = pd.date_range("2026-01-01", periods=3, freq="D")
    c = kittychart.Chart(
        type="line",
        data={
            "labels": days,
            "datasets": [{"data": [1, 2, 3]}, {"data": [{"x": dt.date(2026, 1, 2), "y": 1}]}],
        },
        options={"scales": {"x": {"type": "time"}}},
    )
    d = json.loads(c.to_json())["data"]
    assert [s[:10] for s in d["labels"]] == ["2026-01-01", "2026-01-02", "2026-01-03"]
    assert d["datasets"][1]["data"][0]["x"] == "2026-01-02"
    np_days = np.array(["2026-01-01", "NaT"], dtype="datetime64[ns]")
    c = kittychart.Chart(type="line", data={"labels": np_days, "datasets": [{"data": [1, 2]}]})
    assert json.loads(c.to_json())["data"]["labels"] == ["2026-01-01T00:00:00.000", None]
    c.to_png(200)  # the time scale parses them


@pytest.mark.parametrize("make", [pd.DataFrame, pl.DataFrame])
def test_from_dataframe(make):
    df = make({"day": ["Mon", "Tue"], "a": [1.0, 2.0], "b": [3, 4]})
    c = kittychart.Chart.from_dataframe(df, x="day", type="bar", datasets={"a": {"backgroundColor": "red"}})
    d = json.loads(c.to_json())["data"]
    assert d["labels"] == ["Mon", "Tue"]
    assert [(s["label"], s["data"]) for s in d["datasets"]] == [("a", [1.0, 2.0]), ("b", [3, 4])]
    assert d["datasets"][0]["backgroundColor"] == "red"


def test_from_pandas_uses_the_index():
    df = pd.DataFrame({"v": [1, 2]}, index=["x", "y"])
    d = json.loads(kittychart.Chart.from_dataframe(df).to_json())["data"]
    assert d["labels"] == ["x", "y"] and d["datasets"][0]["data"] == [1, 2]


def test_update_merges_options_and_replaces_data():
    c = kittychart.Chart(BAR, options={"plugins": {"title": {"display": True, "text": "A"}}})
    c.update(options={"plugins": {"title": {"text": "B"}}})
    assert c.config["options"]["plugins"]["title"] == {"display": True, "text": "B"}
    c.update(data={"labels": ["x"], "datasets": [{"label": "New", "data": [1]}]})
    assert "New" in c.to_text(40)
    with pytest.raises(ValueError):
        c.update({"type": "nope"})
    assert c.config["type"] == "bar", "a failed update leaves the chart unchanged"


def test_text_modes():
    c = kittychart.Chart(BAR)
    for mode in ["text", "blocks", "half", "braille"]:
        t = c.to_text(50, 12, mode=mode)
        assert "Revenue" in t and t.count("\r\n") == 12
    with pytest.raises(ValueError):
        c.to_text(40, mode="dots")
    dark = {"background": [0, 0, 0], "foreground": [255, 255, 255], "palette": None}
    assert c.to_text(40, colors=dark) != c.to_text(40)


def test_show_without_a_terminal_prints_text(capsys, monkeypatch):
    # Under pytest there may still be a controlling terminal; force text mode on stdout.
    kittychart.Chart(BAR).show(protocol="text", cols=40)
    out = capsys.readouterr().out
    assert "Revenue" in out and out.endswith("\n")
    with pytest.raises(ValueError):
        kittychart.Chart(BAR).show(protocol="png")
    with pytest.raises(ValueError):
        kittychart.Chart(BAR).show(colors="nope")


def test_module_show_accepts_dicts(capsys):
    kittychart.show(BAR, protocol="braille", cols=30)
    assert "Revenue" in capsys.readouterr().out


def test_repr_png_for_notebooks():
    png = kittychart.Chart(BAR)._repr_png_()
    assert decode_png(png)[:2] == (1600, 800)


def test_version():
    assert kittychart.__version__.count(".") == 2
