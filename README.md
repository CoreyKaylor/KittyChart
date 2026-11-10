# KittyChart

Chart.js-class charts for graphics-capable terminals (kitty, Ghostty and other libghostty
terminals, WezTerm, …), from a Rust core with Python, Node and C bindings.

> Status: **M1, line and bar charts**: category/linear scales, stacking, floating and horizontal
> bars, title/subtitle/legend, rendered in the terminal through the kitty graphics protocol.
> Pie/doughnut/radar/polar/scatter/bubble, log/time scales, and fallbacks for non-kitty terminals
> are next.

```sh
cargo run -p kittychart-cli -- tests/fixtures/line/basic.json          # draw in kitty / Ghostty
cargo run -p kittychart-cli -- chart.json --out chart.png --scale 2   # or render a PNG
echo '{"type":"bar","data":{"labels":["a","b"],"datasets":[{"data":[3,5]}]}}' | cargo run -p kittychart-cli
```

Any Chart.js config works as input: `{ type, data, options }`, with Chart.js's defaults, option
cascading, colors plugin palette and layout rules.

## Workspace

| Crate | Purpose |
|---|---|
| `kittychart` | Facade: `render(config, opts) → RGBA/PNG` |
| `kittychart-core` | Chart.js port: config, option resolution, scales, layout, controllers, plugins |
| `kittychart-cli` | The `kittychart` command |
| `kittychart-canvas` | Canvas2D-shaped drawing API: tiny-skia raster, cosmic-text shaping, bundled Inter |
| `kittychart-term` | Sans-IO kitty encoder and capability `Detector`, plus a small `/dev/tty` adapter |
| `kittychart-testkit` | Independent protocol decoder and golden-image assertions (test-only) |
| `kittychart-conformance` | Tests against Ghostty's real VT core via `libghostty-vt` (test-only, needs Zig) |

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
theme file, so you can preview against your own theme.

Golden fixtures live in `tests/fixtures/**.json` (Chart.js configs, plus an optional
`"_kc": {width, height, dpr, background}`). Each is checked against `tests/goldens/`, and
`just parity` scores it against Chart.js 4.6.1 running in Chromium.

The test layers and the reasoning behind them:

- **Sans-IO**: no library code touches a TTY, env var, clock or system font. Detection is a byte-in,
  capabilities-out state machine, so tests drive it with byte slices or with a real emulator.
- **Goldens** live in `tests/goldens/`. Identical bytes pass immediately. Otherwise a DSSIM
  tolerance absorbs cross-architecture anti-aliasing noise. Failures write
  `actual/expected/diff.png` to `target/kc-diffs/`.
- **Two independent checks of protocol output**: our own spec-based decoder (L2), and Ghostty's
  actual parser and image store running in-process (L3). No terminal window, PTY or permissions.

## License

MIT OR Apache-2.0
