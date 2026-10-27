#!/usr/bin/env bash
# Keep the lead's cargo target directory from growing without bound (it reached 1.3 TB in 2026-10: fast/incremental
# 645 GB, debug 428 GB). Called by tools/lead-run.sh after every command; safe to run by hand.
#   1. incremental session dirs (target/*/incremental/<crate>/s-*) not touched for 2 days go (LEAD_INCR_DAYS);
#      cargo rebuilds a missing session on demand, so only the next build of that crate is slower.
#   2. the debug profile is not part of the lead flow (the shell, installer and apps ship from target/fast); it only
#      appears when a plain `cargo test` runs. Its incremental dir always goes, and the whole debug dir goes once it is
#      over LEAD_DEBUG_MAX_GB (default 30). Anything in it is rebuilt by the next plain test run.
#   3. prints the target size and warns loudly when it is over LEAD_TARGET_WARN_GB (default 300).
# Deletes only below <target>/ (a directory literally named "target"), never anything else.
# usage: tools/target-prune.sh [TARGET_DIR]   (default: springboard/target of this checkout)
set -uo pipefail
T="${1:-$(cd "$(dirname "$0")/.." && pwd)/springboard/target}"
[ -d "$T" ] || { echo "target-prune: no $T, nothing to do"; exit 0; }
T=$(cd "$T" && pwd -P)
[ "$(basename "$T")" = target ] && [ "$T" != / ] || { echo "target-prune: refusing $T (not a target dir)" >&2; exit 2; }
gb() { du -s --block-size=1G "$1" 2>/dev/null | cut -f1; }
before=$(gb "$T")

# 1. stale incremental sessions: <profile>/incremental/<crate>/<session>, also under a target triple dir
find "$T" -mindepth 4 -maxdepth 5 -type d -path '*/incremental/*/s-*' -mtime +"${LEAD_INCR_DAYS:-2}" -prune \
    -exec rm -rf -- {} + 2>/dev/null

# 2. the debug profile
if [ -d "$T/debug" ]; then
    rm -rf -- "$T/debug/incremental"
    if [ "$(gb "$T/debug" || echo 0)" -gt "${LEAD_DEBUG_MAX_GB:-30}" ]; then
        echo "target-prune: debug is over ${LEAD_DEBUG_MAX_GB:-30} GB, removing it"
        rm -rf -- "$T/debug"
    fi
fi

# 3. report
after=$(gb "$T")
echo "target-prune: $T ${before:-?} GB -> ${after:-?} GB"
if [ "${after:-0}" -gt "${LEAD_TARGET_WARN_GB:-300}" ]; then
    echo "!!! target-prune: $T is still ${after} GB (limit ${LEAD_TARGET_WARN_GB:-300}); run 'cargo clean --profile <p>' or remove old profiles by hand" >&2
fi
exit 0
