#!/usr/bin/env bash
# Release gate for an OS candidate (OLI-697). Exit 0 only if every automated check passes.
# Usage: tools/release-check.sh [--skip-tests]   (run from any directory; needs springboard/assets)
set -uo pipefail
cd "$(dirname "$0")/.."
REF=../bifrost-ref
OUT=target/release-check
BLANK_MAX_COLORS=4
JOBS=${JOBS:-4}
fails=0
fail() { echo "FAIL: $*"; fails=$((fails + 1)); }
ok() { echo "ok:   $*"; }

[ -f assets/SFUI.ttf ] || { echo "FAIL: assets/ missing (gitignored; link or copy it into this checkout)"; exit 2; }
[ -d "$REF" ] || { echo "FAIL: $REF missing"; exit 2; }
rm -rf "$OUT" && mkdir -p "$OUT"

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if grep -q "^## $version " CHANGELOG.md; then ok "CHANGELOG has an entry for $version"; else fail "CHANGELOG.md has no '## $version' entry"; fi

if [ "${1:-}" != "--skip-tests" ]; then
  if nice cargo test --workspace --no-fail-fast -j"$JOBS" >"$OUT/tests.log" 2>&1; then ok "cargo test --workspace"; else
    fail "cargo test --workspace (see $OUT/tests.log)"; grep -E "^test .* FAILED|^error" "$OUT/tests.log" | sort -u | head -20
  fi
fi

stamp="$OUT/.stamp"; touch "$stamp"; sleep 1
for src in examples/*_preview.rs examples/preview.rs; do
  name=$(basename "$src" .rs)
  if nice cargo run -q --release -j"$JOBS" --example "$name" >"$OUT/$name.log" 2>&1; then ok "example $name ran"; else fail "example $name exited non-zero (see $OUT/$name.log)"; fi
done

# Every image written by a preview must be non-blank; a preview that draws nothing is a failure.
mapfile -t images < <(find target -maxdepth 3 \( -name '*.ppm' -o -name '*.png' \) -newer "$stamp" -not -path "$OUT/*")
[ "${#images[@]}" -gt 0 ] || fail "previews produced no images"
for img in "${images[@]}"; do
  colors=$(magick "$img" -format %k info: 2>/dev/null || echo 0)
  if [ "$colors" -le "$BLANK_MAX_COLORS" ]; then fail "blank preview ($colors colors): $img"; fi
done
ok "${#images[@]} preview images checked for blankness"

# Reference comparison is visual: side-by-side sheet of the host home screen and the reference bifrost.
if [ -f target/preview.ppm ]; then
  magick "$REF/home.png" -resize x800 target/preview.ppm -resize x800 +append "$OUT/home-vs-bifrost-ref.png" && ok "wrote $OUT/home-vs-bifrost-ref.png (review by eye)"
else fail "target/preview.ppm missing"; fi

echo "$fails failure(s)"; [ "$fails" -eq 0 ]
