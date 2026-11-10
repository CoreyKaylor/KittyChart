# KittyChart developer recipes. `mise install` first to get the pinned toolchain.
set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Fast loop: unit + golden + protocol tests (L0–L2). No Zig needed.
test *args:
    cargo nextest run {{args}}

# Everything, including the libghostty-vt conformance suite (L3). Needs Zig (mise install).
test-all *args:
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

fmt:
    cargo fmt

# What CI's required lanes run, locally.
ci: lint test-all
