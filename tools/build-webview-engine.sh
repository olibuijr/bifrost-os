#!/bin/sh
# Builds the BifrOSt web content engine (bifrost-webview, libservo embedded with hardware GL and explicit CPU fallback) from
# official Servo v0.7.0 source and packages it as the Browser's engine archive. Usage: tools/build-webview-engine.sh VERSION
# Afterwards: put the printed sha256 in springboard/apps/browser/engine.json and its first 12 hex digits in ENGINE
# (springboard/apps/browser/src/main.rs).
# Source: out/servo-source/servo-0.7.0 (unpacked from engine.json's source archive, sha256 checked), with the changes in
# springboard/apps/browser/engine/patches/*.patch applied (springboard/apps/browser/README.md, "Engine").
set -eu
version=${1:?usage: build-webview-engine.sh VERSION}
root=$(cd "$(dirname "$0")/.." && pwd)
here=$root/springboard/apps/browser/engine
src_dir=$root/out/servo-source
src=$src_dir/servo-0.7.0
want=$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["source_archive_sha256"])' "$here/../engine.json")
if [ ! -d "$src" ]; then
  mkdir -p "$src_dir"
  [ -f "$src_dir/servo-v0.7.0.tar.gz" ] || curl -fL -o "$src_dir/servo-v0.7.0.tar.gz" https://github.com/servo/servo/archive/refs/tags/v0.7.0.tar.gz
  echo "$want  $src_dir/servo-v0.7.0.tar.gz" | sha256sum -c -
  tar -xzf "$src_dir/servo-v0.7.0.tar.gz" -C "$src_dir"
fi
# Local changes to Servo itself: engine/patches/*.patch (-p1 from the source root), applied in name order. A copy of
# every applied patch is kept in $src/.bifrost-patches; when the set differs from those copies, the old ones are reversed
# (newest first) and the current set is applied again, so a changed, added or removed patch needs no fresh unpack.
applied=$src/.bifrost-patches
mkdir -p "$applied"
stale=0
for old in "$applied"/*.patch; do
  [ -f "$old" ] || continue
  cmp -s "$old" "$here/patches/$(basename "$old")" || stale=1
done
if [ "$stale" = 1 ]; then
  for old in $(ls -r "$applied"); do
    patch -d "$src" -p1 -R -s < "$applied/$old"
    rm "$applied/$old"
  done
fi
for new in "$here"/patches/*.patch; do
  [ -f "$new" ] || continue
  [ -f "$applied/$(basename "$new")" ] && continue
  patch -d "$src" -p1 -s < "$new"
  cp "$new" "$applied/$(basename "$new")"
done
# the engine itself is one extra workspace member
mkdir -p "$src/ports/bifrost-webview/src"
cp "$here/Cargo.toml" "$src/ports/bifrost-webview/Cargo.toml"
cp "$here"/src/*.rs "$here"/src/*.js "$src/ports/bifrost-webview/src/"
grep -q '"ports/bifrost-webview"' "$src/Cargo.toml" || sed -i 's|    "ports/servoshell",|    "ports/servoshell",\n    "ports/bifrost-webview",|' "$src/Cargo.toml"
(
cd "$src"
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=32 \
CARGO_PROFILE_RELEASE_INCREMENTAL=true CARGO_PROFILE_RELEASE_DEBUG=0 \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='-C link-arg=-fuse-ld=lld' \
cargo +"${BIFROST_ENGINE_TOOLCHAIN:-stable}" build --offline --release -p bifrost-webview -j "${CARGO_BUILD_JOBS:-2}"
)
stage=$(mktemp -d)
mkdir -p "$stage/servo"
cp "$src/target/release/bifrost-webview" "$stage/servo/bifrost-webview"
strip "$stage/servo/bifrost-webview"
cp "$src/LICENSE" "$stage/servo/LICENSE-servo.txt"
out=$root/out/browser-engine/servo-$version.tar.gz
mkdir -p "$root/out/browser-engine"
tar --sort=name --owner=0 --group=0 --mtime=@0 -czf "$out" -C "$stage" servo
rm -rf "$stage"
sha256sum "$out"
