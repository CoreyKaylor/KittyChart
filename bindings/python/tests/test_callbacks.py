import pytest

import kittychart

DATA = [5, 15, 8, 12]


def bar(**dataset):
    return {
        "type": "bar",
        "data": {"labels": ["a", "b", "c", "d"], "datasets": [{"label": "Sales", "data": DATA, **dataset}]},
    }


def test_function_colors_render_like_their_static_values():
    scripted = kittychart.Chart(bar(backgroundColor=lambda ctx: "#ff0000" if ctx.raw > 10 else "#0000ff"))
    static = kittychart.Chart(bar(backgroundColor=["#ff0000" if v > 10 else "#0000ff" for v in DATA]))
    assert scripted.to_rgba(400) == static.to_rgba(400)


def test_context_fields():
    seen = []

    def color(ctx):
        seen.append((ctx.type, ctx.dataIndex, ctx.raw, ctx.parsed.y, ctx["dataset"]["label"], ctx.chart.width))
        return "red"

    kittychart.Chart(bar(backgroundColor=color)).to_png(400)
    assert ("data", 1, 15, 15, "Sales", 400) in seen


def test_tick_and_tooltip_callbacks():
    def money(value, index, ticks):
        assert ticks[index]["value"] == value
        return f"${value:g}"

    c = kittychart.Chart(
        bar(),
        options={
            "scales": {"y": {"ticks": {"callback": money}}},
            "plugins": {"tooltip": {"callbacks": {"label": lambda item: f"{item.label} sold {item.formattedValue}"}}},
        },
    )
    text = c.to_text(60, 20)
    assert "$15" in text and "$0" in text


def test_segment_callbacks():
    c = kittychart.Chart(
        type="line",
        data={"labels": ["a", "b", "c"], "datasets": [{"data": [1, 3, 2], "segment": {
            "borderColor": lambda ctx: "#ff0000" if ctx.p1.parsed.y < ctx.p0.parsed.y else "#00ff00"}}]},
    )
    assert c.to_png(300)[:4] == b"\x89PNG"


def test_errors_in_callbacks_raise():
    def boom(ctx):
        raise ValueError("bad color")

    with pytest.raises(RuntimeError, match="bad color"):
        kittychart.Chart(bar(backgroundColor=boom)).to_png(300)


def test_same_function_twice_is_one_marker():
    f = lambda ctx: "red"  # noqa: E731
    c = kittychart.Chart(bar(backgroundColor=f, borderColor=f))
    assert c.to_json().count('"__kc_fn":"f0"') == 2


def test_whole_numbers_arrive_as_ints_like_javascript_prints_them():
    seen = []

    def label(value, index, ticks):
        seen.append(value)
        return f"${value}"

    c = kittychart.Chart(
        type="bar",
        data={"labels": ["a", "b"], "datasets": [{"data": [5, 15]}]},
        options={"scales": {"y": {"ticks": {"callback": label}}}},
    )
    assert "$15" in c.to_text(cols=50, rows=14)
    assert "$15.0" not in c.to_text(cols=50, rows=14)
    assert all(isinstance(v, int) for v in seen if v == int(v)), seen
