# KittyChart developer recipes. `mise install` first to get the pinned toolchain.
set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Fast loop: unit + golden + protocol tests (L0–L2). No Zig needed.
test *args:
    cargo nextest run {{args}}

# Everything: + Ghostty conformance and bindings end to end (L3). Needs Zig, uv, Node (mise install).
test-all *args: python-dev node-dev
    cargo nextest run --workspace {{args}}

# Only the Ghostty conformance suite.
conformance *args:
    cargo nextest run -p kittychart-conformance {{args}}

# Accept changed golden images and insta snapshots after reviewing target/kc-diffs/.
bless *args:
    KC_BLESS=1 cargo nextest run --workspace --no-fail-fast {{args}} || true
    cargo insta accept --workspace

# Interactively review pending insta snapshots.
review:
    cargo insta review --workspace

# Render every fixture with real Chart.js and KittyChart; open target/parity/report.html.
parity:
    cd tools/reference && npm install --silent --no-audit --no-fund && npx playwright-core install chromium
    node tools/reference/render.mjs tests/fixtures target/parity/ref
    cargo run --quiet -p kittychart --example parity

# Screenshot charts inside simulated terminals (Ghostty VT core) across themes → one contact sheet.
# e.g. `just preview tests/fixtures/bar/stacked.json` or `just preview --theme tokyonight f.json`
preview *args:
    cargo run --quiet -p kittychart-preview -- {{args}}

lint:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings

# Build the Python package into bindings/python/.venv (with the dev tools: pytest, numpy, …).
python-dev:
    cd bindings/python && uv sync --group dev

# Build the Node native module (debug) into bindings/node.
node-dev:
    cd bindings/node && npm ci --no-audit --no-fund && npm run build:debug

# L5: the bindings' own test suites (pytest; node:test + tsc).
test-bindings: python-dev node-dev
    cd bindings/python && .venv/bin/pytest -q
    cd bindings/node && npm test

# Regenerate the C header (crates/kittychart-ffi/include/kittychart.h) from the Rust source.
header:
    KC_BLESS=1 cargo test -q -p kittychart-ffi --test header

# Regenerate the docs gallery (docs/gallery/*.png, docs/gallery.md) from tests/fixtures.
gallery:
    cargo run --quiet --release -p kittychart --example gallery

# Release builds of every distributable, locally: CLI + C library, Python wheel, Node module.
dist:
    cargo build --release -p kittychart-cli -p kittychart-ffi
    cd bindings/python && uvx maturin build --release --out dist
    cd bindings/node && npm ci --no-audit --no-fund && npm run build

fmt:
    cargo fmt

# What CI's required lanes run, locally.
ci: lint test-all test-bindings
