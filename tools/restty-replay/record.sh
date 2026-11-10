#!/usr/bin/env bash
# Record real KittyChart output streams, plus reference screenshots of the same runs, for the
# restty replay page. Each stream is everything the program wrote to its terminal (kc-preview
# runs it on a PTY attached to a simulated Ghostty terminal and answers its capability queries).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
cd "$root"

cargo build -q --release -p kittychart-cli -p kittychart-preview
cargo build -q --release -p kittychart-ratatui --example dashboard

mkdir -p "$here/streams" "$here/reference"
kc="$root/target/release/kittychart"
run() {
  local name=$1
  shift
  target/release/kc-preview --theme tokyonight --cols 100 --rows 30 \
    --sheet "$here/reference/$name.png" --raw "$here/streams/$name.bin" "$@" >/dev/null 2>&1
  echo "$name: $(stat -c%s "$here/streams/$name.bin") bytes"
}

# Inline (one-shot) output: a direct kitty placement, Unicode placeholders, and the text fallback.
run inline-kitty --exec "$kc tests/fixtures/bar/grouped.json --cols 80"
run inline-placeholders --exec "$kc tests/fixtures/line/scales_and_titles.json --cols 80 --placeholders"
run inline-text --exec "$kc tests/fixtures/arc/pie.json --cols 60 --protocol text"
# The interactive viewer: a pointer move (SGR pixel report, mode 1016) onto the Q2 red bar, and
# the "1" key, which hides the first dataset. Stopped at 3 s, before it quits.
run view-hover --exec "$kc view tests/fixtures/hover/bar.json" --keys '1500:\e[<35;750;560M' --at 3000
run view-legend --exec "$kc view tests/fixtures/hover/bar.json" --keys '1500:1' --at 3000
# The ratatui dashboard (Unicode placeholders, double-buffered ids): a pointer move onto "Fri".
run ratatui-dashboard --exec "target/release/examples/dashboard" --keys '1500:\e[<35;32;17M' --at 3000
