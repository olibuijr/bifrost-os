#!/bin/bash
# Run a heavy lead command (cargo build/test, bifrost pc publish, os-verify) in its own memory-limited systemd scope
# at low CPU and I/O priority. Titan has 15 GB; a springboard build can reach ~9 GB, and in the terminal's scope that
# made systemd-oomd kill the whole terminal (2026-10-22). In its own scope the build reclaims memory and slows down
# past MemoryHigh, and at worst only the build is killed at MemoryMax.
# Usage: BIFROST_LEAD_BUILD=1 tools/lead-run.sh COMMAND [ARGS...]
#        LEAD_MEM_HIGH=7G LEAD_MEM_MAX=9G (defaults) to change the limits.
set -euo pipefail
[[ $# -gt 0 ]] || { echo "usage: tools/lead-run.sh COMMAND [ARGS...]" >&2; exit 2; }
exec systemd-run --user --scope --quiet --collect --unit="lead-build-$$" \
    -p MemoryHigh="${LEAD_MEM_HIGH:-7G}" -p MemoryMax="${LEAD_MEM_MAX:-9G}" \
    nice -n 19 ionice -c3 "$@"
