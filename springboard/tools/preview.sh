#!/usr/bin/env bash
# Host preview of the home screen (all text rendered by src/font.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
cargo run -q --release --example preview
magick target/preview.ppm target/preview.png
