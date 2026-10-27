#!/usr/bin/env bash
# Keep the lead's cargo target directory from growing without bound (it reached 1.3 TB in 2026-10: fast/incremental
# 645 GB, debug 428 GB). Called by tools/lead-run.sh after every command; safe to run by hand.
#   1. a crate's whole incremental dir (target/*/incremental/<crate-hash>/) goes once nothing in it changed for 2 days
#      (LEAD_INCR_DAYS): the crate has not been built since. Single sessions of a crate still in use are never touched
#      (2026-10-27: removing old sessions under live crates left an rlib with undefined symbols at link time).
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

# 1. incremental dirs of crates not built for LEAD_INCR_DAYS: <profile>/incremental/<crate-hash>, also under a target
#    triple dir. The newest file decides, so a crate built today keeps all of its sessions.
for d in "$T"/*/incremental/*/ "$T"/*/*/incremental/*/; do
    [ -d "$d" ] || continue
    [ -n "$(find "$d" -newermt "-${LEAD_INCR_DAYS:-2} days" -print -quit 2>/dev/null)" ] || rm -rf -- "$d"
done

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
