#!/bin/bash
# Tab A8: build a springboard binary for the tablet (aarch64 Android), push it and run it: camprobe.sh BIN [args]
# Needs ANDROID_SERIAL; ANDROID_NDK_HOME is optional (defaults to the newest NDK under ~/Android/Sdk).
set -euo pipefail
: "${ANDROID_SERIAL:?set ANDROID_SERIAL to the tablet adb address}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT/springboard"
NDK="${ANDROID_NDK_HOME:-$(ls -d ~/Android/Sdk/ndk/* | tail -1)}"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android29-clang"
BIN=$1; shift
cargo build --release --bin "$BIN" --target aarch64-linux-android --offline 2>&1 | grep -E "^error" -A7 || true
adb -s "$ANDROID_SERIAL" push "target/aarch64-linux-android/release/$BIN" "/data/local/tmp/$BIN" >/dev/null
adb -s "$ANDROID_SERIAL" shell "chmod 755 /data/local/tmp/$BIN; /data/local/tmp/$BIN $* > /data/local/tmp/$BIN.log 2>&1; tail -25 /data/local/tmp/$BIN.log"
