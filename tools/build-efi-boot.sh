#!/usr/bin/env bash
# Build the BifrOSt-owned UEFI boot application.
#
# The firmware still supplies UEFI (or legacy firmware's CSM); everything after
# the firmware handoff is the Rust BifrOSt loader. Keep the generated binary
# out of Cargo's target directory so the ISO and recovery package consume the
# same artifact.
set -euo pipefail
cd "$(dirname "$0")/.."

PROFILE=${1:-release}
TARGET=x86_64-unknown-uefi

cargo build --offline --manifest-path springboard/bootweave/efi/Cargo.toml \
  --target "$TARGET" --profile "$PROFILE"

BOOT="springboard/bootweave/efi/target/$TARGET/$PROFILE/bootweave-efi.efi"
test -s "$BOOT" || { echo "missing EFI output: $BOOT" >&2; exit 1; }
install -p -m 0755 "$BOOT" packaging/archiso/bootweave.efi
install -p -m 0755 "$BOOT" packaging/arch/recovery/bootweave.efi
# never leave a loader older than its source
tools/verify-efi-loader.sh "$BOOT" packaging/archiso/bootweave.efi packaging/arch/recovery/bootweave.efi
printf 'BifrOSt EFI boot: %s\n' "$BOOT"
