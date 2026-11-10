#!/usr/bin/env bash
# Build the demo site into target/web (or $OUT).
#
#   web/build.sh                 the demo, and the docs (docs/, an mdBook) under /docs
#   KC_REPO_URL=https://… …      link the header's GitHub item and the docs' source links to the
#                                repository (KC_REPO_REF: the commit they point at, default main)
#   DOCS_SITE=dir web/build.sh   use an already-built docs site instead
#
# Needs: the wasm32-unknown-unknown target, wasm-bindgen-cli matching the crate's pinned
# wasm-bindgen (see crates/kittychart-wasm/Cargo.toml), mdbook (mise.toml), npm, curl.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
out=${OUT:-$root/target/web}
cache=$root/target/web-cache
mkdir -p "$cache"

want_bindgen=$(sed -n 's/^wasm-bindgen = "=\(.*\)"/\1/p' "$root/crates/kittychart-wasm/Cargo.toml")
have_bindgen=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}')
if [ "$have_bindgen" != "$want_bindgen" ]; then
  echo "error: need wasm-bindgen-cli $want_bindgen (have: ${have_bindgen:-none})" >&2
  echo "  cargo install wasm-bindgen-cli --version $want_bindgen --locked" >&2
  exit 1
fi

echo "==> wasm"
# SIMD speeds up tiny-skia's rasterizer (every browser with WebAssembly SIMD: all current ones).
RUSTFLAGS="-C target-feature=+simd128" cargo build --release -p kittychart-wasm \
  --target wasm32-unknown-unknown --target-dir "$root/target/wasm"
rm -rf "$out"
mkdir -p "$out/pkg" "$out/vendor" "$out/fonts"
wasm-bindgen --target web --no-typescript --out-dir "$out/pkg" \
  "$root/target/wasm/wasm32-unknown-unknown/release/kittychart_wasm.wasm"

echo "==> restty"
(cd "$here" && { [ -d node_modules/restty ] && [ node_modules/.package-lock.json -nt package-lock.json ] || npm ci --silent; })
cp "$here/node_modules/restty/dist/restty.esm.js" "$out/vendor/"
cp "$here/node_modules/restty/LICENSE" "$out/vendor/restty-LICENSE.txt"

echo "==> fonts"
fetch() { # url sha256 dest
  if [ ! -f "$cache/$3" ] || ! echo "$2  $cache/$3" | sha256sum -c --quiet - 2>/dev/null; then
    curl -sfL --retry 3 -o "$cache/$3.tmp" "$1"
    echo "$2  $cache/$3.tmp" | sha256sum -c --quiet -
    mv "$cache/$3.tmp" "$cache/$3"
  fi
  cp "$cache/$3" "$out/fonts/$3"
}
jbm=https://cdn.jsdelivr.net/gh/JetBrains/JetBrainsMono@v2.304
fetch "$jbm/fonts/ttf/JetBrainsMonoNL-Regular.ttf" fb3b2575d7b0657359707993288f12a7360344d39387bb26050e276d61f6bd2a JetBrainsMono-Regular.ttf
fetch "$jbm/fonts/ttf/JetBrainsMonoNL-Bold.ttf" 0198e841824025f8876e5c297f0b9b497ee8d6eb9969710a3328e1303f996ec3 JetBrainsMono-Bold.ttf
fetch "$jbm/OFL.txt" 30f0c136e3c88e422d0791acd97238870f9054a9729bc34cf2ff0d4ed8cac4ad JetBrainsMono-OFL.txt
# Inter: the files the charts embed.
cp "$root"/crates/kittychart-canvas/fonts/Inter-{Regular.ttf,Bold.ttf,OFL.txt} "$out/fonts/"

echo "==> site"
cp "$here/index.html" "$here/style.css" "$here/favicon.svg" "$out/"
cp -r "$here/js" "$here/configs" "$out/"
if [ -n "${KC_REPO_URL:-}" ]; then
  sed -i "s#<meta name=\"kc-repo\" content=\"\">#<meta name=\"kc-repo\" content=\"${KC_REPO_URL}\">#" "$out/index.html"
fi

echo "==> docs"
if [ -n "${DOCS_SITE:-}" ]; then
  cp -r "$DOCS_SITE" "$out/docs"
else
  "$here/docs.sh" "$out/docs"
fi

wasm=$(stat -c%s "$out/pkg/kittychart_wasm_bg.wasm")
gz=$(gzip -9c "$out/pkg/kittychart_wasm_bg.wasm" | wc -c)
echo "built $out (wasm $((wasm / 1024)) KB, $((gz / 1024)) KB gzipped)"
