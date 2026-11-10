# KittyChart

Chart.js-class charts for graphics-capable terminals (kitty, Ghostty and other libghostty
terminals, WezTerm, …), from a Rust core with Python, Node and C bindings.

> Status: **M3 complete**: line, bar, scatter, bubble, pie, doughnut, radar and polar area
> charts; category, linear, logarithmic, time, timeseries and radial scales; stacking, floating
> and horizontal bars; area fills; decimation; title/subtitle/legend; terminal-native colors.
> Output via kitty graphics (direct or Unicode placeholders, tmux passthrough), sixel, or text
> (block and Braille characters), plus a ratatui widget. Next (M4): C, Python and Node bindings.

```sh
cargo run -p kittychart-cli -- tests/fixtures/line/basic.json          # draw in kitty / Ghostty
cargo run -p kittychart-cli -- chart.json --out chart.png --scale 2   # or render a PNG
echo '{"type":"bar","data":{"labels":["a","b"],"datasets":[{"data":[3,5]}]}}' | cargo run -p kittychart-cli
cargo run -p kittychart-cli -- chart.json --protocol text             # no graphics? block + Braille text
```

`--protocol auto` (the default, or `$KITTYCHART_PROTOCOL`) asks the terminal what it supports
and picks kitty graphics, then sixel, then text. Inside tmux, kitty images go through
passthrough as Unicode placeholders, so tmux keeps them in their pane (needs
`set -g allow-passthrough on`). In text mode, labels are real characters, regions (bars,
areas, slices) are quadrant blocks and lines are Braille.

Any Chart.js config works as input: `{ type, data, options }`, with Chart.js's defaults, option
cascading, colors plugin palette and layout rules.

## Workspace

| Crate | Purpose |
|---|---|
| `kittychart` | Facade: `render(config, opts) → RGBA/PNG` |
| `kittychart-core` | Chart.js port: config, option resolution, scales, layout, controllers, plugins |
| `kittychart-cli` | The `kittychart` command |
| `kittychart-ratatui` | ratatui widget: kitty graphics through Unicode placeholders, or text cells |
| `kittychart-canvas` | Canvas2D-shaped drawing API: tiny-skia raster, cosmic-text shaping, bundled Inter |
| `kittychart-term` | Sans-IO kitty (incl. placeholders, tmux) and sixel encoders, capability `Detector`, a small `/dev/tty` adapter |
| `kittychart-testkit` | Independent kitty and sixel decoders, golden-image assertions (test-only) |
| `kittychart-conformance` | Tests against reference implementations: Ghostty's VT core (`libghostty-vt`, needs Zig), libsixel, tmux; plus a PTY harness |

## Development

```sh
mise install        # pinned toolchain: zig, just, nextest, insta, node, uv
just test           # L0–L2: unit, golden images, protocol round-trips (no Zig needed)
just test-all       # + L3 Ghostty conformance
just bless          # accept changed goldens/snapshots (review target/kc-diffs/ first)
just ci             # fmt + clippy + test-all, as CI runs them
just parity         # render fixtures with real Chart.js (headless Chromium) and diff them
just preview FILE…  # screenshot charts inside simulated terminals across themes → target/preview/sheet.png
```

`just preview` is the look-and-feel check. It replays the real CLI session (prompt, capability
and theme detection, kitty graphics) inside Ghostty's VT core for several terminal themes, then
paints each terminal's screen into one contact sheet. `--theme` takes built-in names or a Ghostty
theme file, so you can preview against your own theme. `--protocol` previews the other outputs:
`placeholders`, `sixel` (decoded by the testkit, since Ghostty has no sixel), and the text modes
`auto`, `blocks`, `half`, `braille`.

`--exec` runs any real program on a PTY attached to the simulated terminal and screenshots it,
e.g. `just preview --exec "target/debug/kittychart chart.json"`, the ratatui example
(`--exec target/debug/examples/dashboard --keys 2000:q`), or either inside tmux.
`--no-graphics` simulates a terminal without kitty graphics.

Golden fixtures live in `tests/fixtures/**.json` (Chart.js configs, plus an optional
`"_kc": {width, height, dpr, background}`). Each is checked against `tests/goldens/`, and
`just parity` scores it against Chart.js 4.6.1 running in Chromium.

The test layers and the reasoning behind them:

- **Sans-IO**: no library code touches a TTY, env var, clock or system font. Detection is a byte-in,
  capabilities-out state machine, so tests drive it with byte slices or with a real emulator.
- **Goldens** live in `tests/goldens/`. Identical bytes pass immediately. Otherwise a DSSIM
  tolerance absorbs cross-architecture anti-aliasing noise. Failures write
  `actual/expected/diff.png` to `target/kc-diffs/`.
- **Independent checks of protocol output**: our own spec-based decoders (L2), then reference
  implementations (L3): Ghostty's actual parser and image store running in-process, and
  libsixel's decoder (`sixel2png`). No terminal window or permissions.
- **End to end** (L3): the real `kittychart` binary, the ratatui example, and tmux, each run on a
  PTY whose other end is Ghostty's VT core, which answers their queries. This covers the TTY
  adapter, detection, protocol choice, cursor placement and tmux passthrough. The conformance
  lane needs Zig, `sixel2png` (libsixel) and tmux.

## License

MIT OR Apache-2.0
