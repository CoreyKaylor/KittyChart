import json
import struct
import zlib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = ROOT / "tests" / "fixtures"
GOLDENS = ROOT / "tests" / "goldens" / "fixtures"


def decode_png(data: bytes):
    """A minimal decoder for the 8-bit RGBA PNGs the goldens use: (width, height, rgba)."""
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    pos, idat, width = 8, b"", 0
    while pos < len(data):
        (n,) = struct.unpack(">I", data[pos : pos + 4])
        kind, body = data[pos + 4 : pos + 8], data[pos + 8 : pos + 8 + n]
        if kind == b"IHDR":
            width, height, depth, color = struct.unpack(">IIBB", body[:10])
            assert (depth, color) == (8, 6), "8-bit RGBA only"
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw = zlib.decompress(idat)
    stride = width * 4
    out = bytearray()
    prev = bytearray(stride)
    for y in range(height):
        f = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - 4] if i >= 4 else 0
            b = prev[i]
            c = prev[i - 4] if i >= 4 else 0
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + ((a + b) >> 1)) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else b if pb <= pc else c
                line[i] = (line[i] + pred) & 255
        out += line
        prev = line
    return width, height, bytes(out)


@pytest.fixture
def fixture_config():
    def load(name: str):
        cfg = json.loads((FIXTURES / name).read_text())
        return cfg, cfg.pop("_kc", {})

    return load


@pytest.fixture(autouse=True)
def no_terminal(monkeypatch):
    """Tests never probe a real terminal: pretend there is none unless a test sets one."""
    import kittychart._chart as c

    monkeypatch.setattr(c, "_INFO", None)
    monkeypatch.setattr(c, "_PROBED", True)
    monkeypatch.delenv("KITTYCHART_PROTOCOL", raising=False)
