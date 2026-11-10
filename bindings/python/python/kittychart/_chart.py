from __future__ import annotations

import datetime as _dt
import decimal
import json
import math
import os
import sys
from typing import Any, Iterable, Mapping, Optional, Sequence, Union

from . import _native

__all__ = ["Chart", "show", "probe", "terminal_info"]

Config = Mapping[str, Any]


# --- converting Python data to Chart.js JSON -------------------------------------------------


def _jsonable(obj: Any, fns: Optional[dict] = None) -> Any:
    """Convert dicts/lists of Python, numpy, pandas and polars values into JSON-ready values.

    NaN and infinities become ``null`` (Chart.js draws them as gaps), dates become ISO 8601
    strings (which time scales parse), numpy/pandas/polars arrays become lists. Functions
    (scriptable options) become ``{"__kc_fn": id}`` markers, registered in ``fns``.
    """
    if obj is None or isinstance(obj, (str, bool, int)):
        return obj
    if callable(obj) and not isinstance(obj, type) and not hasattr(obj, "tolist"):
        if fns is None:
            raise TypeError("functions are only allowed in chart configs (scriptable options)")
        for k, f in fns.items():
            if f is obj:
                return {"__kc_fn": k}
        k = f"f{len(fns)}"
        fns[k] = obj
        return {"__kc_fn": k}
    if isinstance(obj, float):
        return obj if math.isfinite(obj) else None
    if isinstance(obj, Mapping):
        return {str(k): _jsonable(v, fns) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return [_jsonable(v, fns) for v in obj]
    if isinstance(obj, (_dt.datetime, _dt.date)):
        # pandas.NaT is a datetime subclass whose isoformat() is "NaT".
        s = obj.isoformat()
        return None if s == "NaT" else s
    if isinstance(obj, decimal.Decimal):
        return _jsonable(float(obj))
    dtype = getattr(obj, "dtype", None)
    kind = getattr(dtype, "kind", None)
    if kind == "M" and hasattr(obj, "astype") and not hasattr(obj, "dt"):
        # numpy datetime64 (ns values would otherwise become plain ints).
        return [None if s == "NaT" else s for s in _tolist(obj.astype("datetime64[ms]").astype(str))]
    if hasattr(obj, "tolist") or hasattr(obj, "to_list"):
        values = _tolist(obj)
        if kind in ("i", "u", "b"):
            return values  # no NaN or dates possible
        if kind == "f" and not isinstance(values, list):
            return _jsonable(values)  # numpy scalar
        if kind == "f" and all(v == v and v not in (math.inf, -math.inf) for v in values):
            return values  # the fast path for clean float arrays
        return _jsonable(values)
    if hasattr(obj, "item"):  # other numpy scalars
        return _jsonable(obj.item())
    if isinstance(obj, Iterable):
        return [_jsonable(v) for v in obj]
    raise TypeError(f"kittychart can't convert {type(obj).__name__} to chart data")


def _tolist(obj: Any) -> Any:
    if hasattr(obj, "to_list"):  # polars
        return obj.to_list()
    return obj.tolist()


def _merge(base: dict, patch: Mapping) -> dict:
    out = dict(base)
    for k, v in patch.items():
        if isinstance(v, Mapping) and isinstance(out.get(k), dict):
            out[k] = _merge(out[k], v)
        else:
            out[k] = v
    return out


# --- terminal detection ----------------------------------------------------------------------

_INFO: Optional[dict] = None
_PROBED = False


def probe(timeout: float = 0.25) -> Optional[dict]:
    """Ask the terminal what it supports (and remember it for :func:`terminal_info`).

    Returns a dict with ``cols``, ``rows``, ``cell_width``, ``cell_height`` (device pixels),
    ``kitty``, ``sixel``, ``tmux`` and ``colors`` (``{"background", "foreground",
    "palette"}`` or ``None``), or ``None`` when there's no terminal.
    """
    global _INFO, _PROBED
    try:
        _INFO = _native.probe(int(timeout * 1000))
    except OSError:
        _INFO = None
    _PROBED = True
    return _INFO


def terminal_info() -> Optional[dict]:
    """The result of :func:`probe`, probing on first use."""
    return _INFO if _PROBED else probe()


def _in_jupyter() -> bool:
    ip = sys.modules.get("IPython")
    if ip is None or "ipykernel" not in sys.modules:
        return False
    shell = ip.get_ipython()
    return shell is not None and type(shell).__name__ == "ZMQInteractiveShell"


def _write(prepared: Any) -> None:
    if prepared.target == "tty":
        _native.write_tty(prepared.data)
        return
    out = sys.stdout
    buf = getattr(out, "buffer", None)
    if buf is not None:
        out.flush()
        buf.write(prepared.data)
        buf.flush()
    else:
        out.write(prepared.data.decode("utf-8"))
        out.flush()


def _show_options(
    protocol: Optional[str] = None,
    cols: Optional[int] = None,
    rows: Optional[int] = None,
    width: Optional[float] = None,
    height: Optional[float] = None,
    scale: Optional[float] = None,
    background: Optional[str] = None,
    placeholders: bool = False,
    force: bool = False,
    colors: str = "terminal",
    image_id: Optional[int] = None,
) -> dict:
    if colors not in ("terminal", "chartjs"):
        raise ValueError('colors must be "terminal" or "chartjs"')
    return {
        "protocol": protocol,
        "cols": cols,
        "rows": rows,
        "width": width,
        "height": height,
        "scale": scale,
        "background": background,
        "placeholders": placeholders,
        "force": force,
        "chartjs_colors": colors == "chartjs",
        "image_id": image_id,
    }


# --- the chart -------------------------------------------------------------------------------


class Context(dict):
    """A Chart.js option context (or tooltip item) for scriptable options: a dict that also
    allows attribute access, so ``ctx.raw``, ``ctx.parsed.y`` and ``ctx["dataIndex"]`` all
    work."""

    def __getattr__(self, name: str) -> Any:
        try:
            return self[name]
        except KeyError:
            raise AttributeError(name) from None


def _wrap(v: Any) -> Any:
    if isinstance(v, dict):
        return Context({k: _wrap(x) for k, x in v.items()})
    if isinstance(v, list):
        return [_wrap(x) for x in v]
    return v


def _dispatcher(fns: dict) -> Any:
    """The native side's callback: ``(id, context_json) -> result_json``."""

    def dispatch(fid: str, ctx_json: str) -> str:
        ctx = json.loads(ctx_json)
        args = ctx.pop("__args", None)
        f = fns[fid]
        result = f(*[_wrap(a) for a in args]) if args is not None else f(_wrap(ctx))
        return json.dumps(_jsonable(result), allow_nan=False)

    return dispatch


class Chart:
    """A Chart.js chart: ``Chart({"type": ..., "data": ..., "options": ...})`` or
    ``Chart(type="bar", data={...}, options={...})``.

    Data may contain numpy arrays, pandas/polars Series, datetimes and NaN (drawn as a gap).
    Options may be functions, as in Chart.js: they get the option's context (a
    :class:`Context`: ``ctx.raw``, ``ctx.parsed.y``, ``ctx.dataIndex`` …) or, where Chart.js
    passes arguments (``ticks.callback(value, index, ticks)``, ``tooltip.callbacks.label(item)``),
    those arguments.
    """

    def __init__(
        self,
        config: Optional[Config] = None,
        *,
        type: Optional[str] = None,
        data: Optional[Config] = None,
        options: Optional[Config] = None,
    ) -> None:
        cfg: dict = dict(config) if config is not None else {}
        if type is not None:
            cfg["type"] = type
        if data is not None:
            cfg["data"] = data
        if options is not None:
            cfg["options"] = options
        self._set(cfg)

    def _set(self, cfg: dict) -> None:
        fns: dict = {}
        text = json.dumps(_jsonable(cfg, fns), allow_nan=False, separators=(",", ":"))
        _native.validate(text)  # raises ValueError with the reason
        self._dispatch = _dispatcher(fns) if fns else None
        self._config = cfg
        self._json = text
        # kitty image id for inline rendering (Rich), reused while the config is unchanged so
        # re-renders (e.g. rich.live) replace the image instead of adding one per frame. A new
        # config gets a new id, so charts printed earlier keep showing their own data.
        self._image_id: Optional[int] = None

    def _inline_image_id(self) -> int:
        if self._image_id is None:
            self._image_id = _native.next_image_id()
        return self._image_id

    @classmethod
    def from_json(cls, text: str) -> "Chart":
        return cls(json.loads(text))

    @classmethod
    def from_dataframe(
        cls,
        df: Any,
        x: Optional[str] = None,
        y: Union[str, Sequence[str], None] = None,
        *,
        type: str = "line",
        options: Optional[Config] = None,
        datasets: Optional[Mapping[str, Config]] = None,
    ) -> "Chart":
        """One dataset per ``y`` column, labelled by ``x`` (default: the pandas index, or row
        numbers). Works with pandas and polars. ``datasets`` adds per-column dataset options,
        e.g. ``{"revenue": {"fill": True}}``."""
        columns = list(df.columns)
        if isinstance(y, str):
            y = [y]
        if y is None:
            y = [c for c in columns if c != x]
        if x is not None:
            labels = df[x]
        elif hasattr(df, "index"):
            labels = df.index
        else:
            labels = list(range(len(df)))
        extra = datasets or {}
        sets = [{"label": str(c), "data": df[c], **extra.get(c, {})} for c in y]
        return cls(type=type, data={"labels": labels, "datasets": sets}, options=options or {})

    @property
    def config(self) -> dict:
        """The configuration (as given; numpy etc. values aren't converted)."""
        return self._config

    def to_json(self) -> str:
        """The configuration as Chart.js JSON."""
        return self._json

    def update(
        self,
        config: Optional[Config] = None,
        *,
        data: Optional[Config] = None,
        options: Optional[Config] = None,
    ) -> "Chart":
        """Merge changes into the configuration (nested dicts merge; lists and values replace).
        ``data`` replaces the data entirely. Returns the chart."""
        cfg = _merge(self._config, config or {})
        if data is not None:
            cfg["data"] = data
        if options is not None:
            cfg["options"] = _merge(cfg.get("options", {}), options)
        self._set(cfg)
        return self

    # Rendering to images and text.

    def to_png(
        self,
        width: float = 800,
        height: Optional[float] = None,
        *,
        scale: float = 1.0,
        background: Optional[str] = None,
    ) -> bytes:
        """PNG bytes. ``width``/``height`` are CSS pixels (height defaults to the aspect ratio);
        ``scale`` is the device pixel ratio."""
        return _native.render_png(self._json, float(width), _f(height), float(scale), background, self._dispatch)

    def to_svg(
        self,
        width: float = 800,
        height: Optional[float] = None,
        *,
        scale: float = 1.0,
        background: Optional[str] = None,
    ) -> str:
        """An SVG document of the same chart. Text is drawn as outlines (no fonts needed)."""
        return _native.render_svg(self._json, float(width), _f(height), float(scale), background, self._dispatch)

    def save(self, path: Union[str, os.PathLike], **kwargs: Any) -> None:
        """Write a PNG file, or an SVG file when the name ends in ``.svg`` (see
        :meth:`to_png` for options)."""
        if str(path).lower().endswith(".svg"):
            with open(path, "w", encoding="utf-8") as f:
                f.write(self.to_svg(**kwargs))
            return
        with open(path, "wb") as f:
            f.write(self.to_png(**kwargs))

    def to_rgba(
        self,
        width: float = 800,
        height: Optional[float] = None,
        *,
        scale: float = 1.0,
        background: Optional[str] = None,
    ) -> "tuple[int, int, bytes]":
        """``(width, height, pixels)``: non-premultiplied RGBA8, row by row."""
        return _native.render_rgba(self._json, float(width), _f(height), float(scale), background, self._dispatch)

    def to_text(
        self,
        cols: int = 80,
        rows: Optional[int] = None,
        *,
        mode: str = "text",
        colors: Optional[Mapping[str, Any]] = None,
    ) -> str:
        """Block and Braille characters with truecolor ANSI escapes. ``mode`` is "text",
        "blocks", "half" or "braille"; ``colors`` like ``probe()["colors"]``."""
        return _native.render_text(self._json, cols, rows, mode, dict(colors) if colors else None, self._dispatch)

    # Showing in the terminal.

    def show(
        self,
        *,
        protocol: Optional[str] = None,
        cols: Optional[int] = None,
        rows: Optional[int] = None,
        width: Optional[float] = None,
        height: Optional[float] = None,
        scale: Optional[float] = None,
        background: Optional[str] = None,
        placeholders: bool = False,
        force: bool = False,
        colors: str = "terminal",
    ) -> None:
        """Draw the chart at the cursor: kitty graphics, then sixel, then text, whichever the
        terminal supports (``protocol`` forces one: "kitty", "sixel", "text", "blocks",
        "half", "braille"; default ``$KITTYCHART_PROTOCOL`` or "auto"). In Jupyter, displays
        the PNG instead."""
        if _in_jupyter() and protocol is None:
            from IPython.display import display

            display(self)
            return
        opts = _show_options(
            protocol, cols, rows, width, height, scale, background, placeholders, force, colors
        )
        try:
            sys.stdout.flush()
        except Exception:
            pass
        _write(_native.prepare(self._json, opts, self._dispatch))

    def interact(
        self,
        *,
        watch: Optional[Union[str, os.PathLike]] = None,
        protocol: Optional[str] = None,
        scale: Optional[float] = None,
        background: Optional[str] = None,
        colors: str = "terminal",
        fast_transfer: bool = True,
    ) -> None:
        """Explore the chart full-screen until ``q``: hover for tooltips, click legend items
        to hide datasets, arrows to step through the data, ``r`` to replay the animation,
        ``s`` to save a PNG. With ``watch``, the chart animates to that JSON file's contents
        whenever it changes."""
        if colors not in ("terminal", "chartjs"):
            raise ValueError('colors must be "terminal" or "chartjs"')
        opts = {
            "protocol": protocol,
            "scale": scale,
            "background": background,
            "chartjs_colors": colors == "chartjs",
            "no_fast_transfer": not fast_transfer,
            "watch": os.fspath(watch) if watch is not None else None,
        }
        try:
            sys.stdout.flush()
        except Exception:
            pass
        _native.interact(self._json, opts, self._dispatch)

    # Notebook and IPython integration.

    def _repr_png_(self) -> bytes:
        return self.to_png(scale=2)

    def _repr_pretty_(self, p: Any, cycle: bool) -> None:
        # Terminal IPython: displaying a chart draws it. (Jupyter shows the PNG instead.)
        if cycle or _in_jupyter():
            p.text(repr(self))
            return
        prepared = _native.prepare(self._json, _show_options(), self._dispatch)
        if prepared.target == "tty":
            _native.write_tty(prepared.data)
            p.text(repr(self))
        else:
            # IPython prints this after "Out[n]:", on a line of its own only if the text has a
            # newline. A kitty image escape alone has none (it draws at the cursor), so keep
            # its trailing newline then.
            text = prepared.data.decode("utf-8")
            stripped = text.rstrip("\n")
            p.text(stripped if "\n" in stripped else text)

    def __repr__(self) -> str:
        n = len(self._config.get("data", {}).get("datasets", []) or [])
        return f"<kittychart.Chart type={self._config.get('type')!r} datasets={n}>"

    # Rich integration (rich.print(chart), Console.print, panels, tables …).

    def __rich_console__(self, console: Any, options: Any) -> Any:
        from ._rich import render

        return render(self, console, options)

    def __rich_measure__(self, console: Any, options: Any) -> Any:
        from rich.measure import Measurement

        return Measurement(min(10, options.max_width), options.max_width)


def _f(v: Optional[float]) -> Optional[float]:
    return None if v is None else float(v)


def show(config: Union[Config, Chart], **kwargs: Any) -> None:
    """Draw a Chart.js config (or a :class:`Chart`) in the terminal. See :meth:`Chart.show`."""
    (config if isinstance(config, Chart) else Chart(config)).show(**kwargs)
