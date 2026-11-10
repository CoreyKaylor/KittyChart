#!/usr/bin/env bash
# Build the docs (docs/, an mdBook) into OUT for the site's /docs/. Called by web/build.sh.
#
#   web/docs.sh OUT
#   KC_REPO_URL=https://github.com/o/r [KC_REPO_REF=<sha>] web/docs.sh OUT
#       links into the source tree (../crates/…, ../tests/…) point at the repository
#       (without KC_REPO_URL they stay relative, which only resolves in a checkout)
#
# The book is built from a copy (target/docs-src) so the source in docs/ stays readable on
# GitHub as plain Markdown.
set -euo pipefail

out=${1:?usage: web/docs.sh OUT}
root=$(cd "$(dirname "$0")/.." && pwd)
src=$root/target/docs-src

want=$(sed -n 's/^"cargo:mdbook" = "\(.*\)".*/\1/p' "$root/mise.toml")
have=$(mdbook --version 2>/dev/null | sed 's/^mdbook v//')
if [ -z "$have" ]; then
  echo "error: mdbook not found (mise install, or cargo install mdbook --version $want --locked)" >&2
  exit 1
fi
[ "$have" = "$want" ] || echo "warning: mdbook $have, the site is built with $want (mise.toml)" >&2

rm -rf "$src"
cp -r "$root/docs" "$src"
# The binding pages include the READMEs from the repository.
sed -i "s#{{\#include \.\./bindings/#{{\#include $root/bindings/#" "$src/python.md" "$src/node.md"
# A way back to the demo, the site's front page.
sed -i '1a\\n<a class="kc-demo-link" href="../">← Live demo: the same library in a browser canvas and in a terminal</a>' "$src/index.md"
if [ -n "${KC_REPO_URL:-}" ]; then
  ref=${KC_REPO_REF:-main}
  sed -i "s#](\.\./\(crates\|tests\)/#](${KC_REPO_URL}/blob/${ref}/\1/#g" "$src"/*.md
  export MDBOOK_OUTPUT__HTML__GIT_REPOSITORY_URL="$KC_REPO_URL"
fi

rm -rf "$out"
mdbook build "$src" -d "$out"
# Only the book's own files: drop the copied sources mdBook passes through.
rm -f "$out/book.toml" "$out/SUMMARY.md"
