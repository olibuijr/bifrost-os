#!/usr/bin/env bash
# Refuse a stale EFI loader: every FILE given must be newer than (or as new as) the newest file under
# springboard/bootweave/{efi,frame}/src (and the two Cargo.toml). usage: tools/verify-efi-loader.sh FILE...
# Callers build with tools/build-efi-boot.sh first (it installs with `install -p`, so copies keep the build mtime).
set -euo pipefail
cd "$(dirname "$0")/.."
newest=$(find springboard/bootweave/efi/src springboard/bootweave/frame/src springboard/bootweave/efi/Cargo.toml \
  springboard/bootweave/frame/Cargo.toml -type f -printf '%T@ %p\n' | sort -n | tail -1)
[ -n "$newest" ] || { echo "verify-efi-loader: no loader sources found" >&2; exit 1; }
for f in "$@"; do
  [ -s "$f" ] || { echo "verify-efi-loader: $f missing or empty (run tools/build-efi-boot.sh)" >&2; exit 1; }
  if [ "$(stat -c %Y "$f")" -lt "$(printf '%.0f' "${newest%% *}")" ]; then
    echo "verify-efi-loader: STALE $f is older than ${newest#* }; run tools/build-efi-boot.sh" >&2
    exit 1
  fi
done
