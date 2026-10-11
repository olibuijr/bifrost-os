#!/bin/sh
# Builds the BifrOSt web content engine (bifrost-webview, libservo embedded with a software GL context) from
# official Servo v0.7.0 source and packages it as the Browser's engine archive. Usage: tools/build-webview-engine.sh VERSION
# Afterwards: put the printed sha256 in springboard/apps/browser/engine.json and its first 12 hex digits in ENGINE
# (springboard/apps/browser/src/main.rs).
# Source: out/servo-source/servo-0.7.0 (unpacked from engine.json's source archive, sha256 checked).
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
# the engine is one extra workspace member; Servo itself is unmodified
mkdir -p "$src/ports/bifrost-webview/src"
cp "$here/Cargo.toml" "$src/ports/bifrost-webview/Cargo.toml"
cp "$here"/src/*.rs "$here"/src/*.js "$src/ports/bifrost-webview/src/"
grep -q '"ports/bifrost-webview"' "$src/Cargo.toml" || sed -i 's|    "ports/servoshell",|    "ports/servoshell",\n    "ports/bifrost-webview",|' "$src/Cargo.toml"
CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=32 \
CARGO_PROFILE_RELEASE_INCREMENTAL=true CARGO_PROFILE_RELEASE_DEBUG=0 \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS='-C link-arg=-fuse-ld=lld' \
cargo +1.98.1 build --offline --release --manifest-path "$src/Cargo.toml" -p bifrost-webview
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
