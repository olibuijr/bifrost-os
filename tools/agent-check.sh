#!/bin/bash
# The one compile check agents may run (owner, 2026-10-25: "allow cargo check"): `cargo check` only (no linking, no
# tests), in its own memory-limited systemd scope at the lowest CPU and I/O priority, one check at a time on Titan,
# with a shared target directory so dependencies are checked once for every worktree.
#
# Stale-artifact guard: cargo names workspace crates by their workspace-relative path, so every worktree maps to the
# same artifacts, and a crate counts as fresh when no source file is newer than the stored check. A sibling worktree's
# older build therefore looked fresh to a worktree whose files are older (bogus errors such as a constant "not found"
# that the sources define). A private target per worktree costs many GB, and checksum freshness needs a nightly
# toolchain. Instead the target records which checkout last used it (`.owner`); when a different one runs (under the
# lock, so runs never overlap), every file of the checkout is touched, which makes this checkout's crates newer than
# any stored check and rebuilds just the workspace crates once. Registry dependencies are keyed by version and keep
# their artifacts. Consecutive runs in the same checkout stay incremental. Source files only get a new mtime.
# Warnings (dead code included) and errors are printed as short-format lines; the output is capped at 300 lines.
# Usage (from anywhere inside a bifrost-os checkout or worktree):
#   tools/agent-check.sh                 the shell: lib + tests without desktop-wayland, then the shell bin with it
#   tools/agent-check.sh -p bifrost-wm   any other `cargo check` arguments instead of the default pair
set -uo pipefail
root=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "agent-check: not inside a git checkout" >&2; exit 2; }
cd "$root/springboard" || { echo "agent-check: no springboard/ in $root" >&2; exit 2; }
export CARGO_TARGET_DIR="${AGENT_CHECK_TARGET:-$HOME/.cache/bifrost-agent-check}"
mkdir -p "$CARGO_TARGET_DIR"
lock="$CARGO_TARGET_DIR/.agent-check.lock"
run() {
    systemd-run --user --scope --quiet --collect --unit="agent-check-$$-$RANDOM" \
        -p MemoryHigh=3G -p MemoryMax=4G nice -n 19 ionice -c3 \
        cargo check --offline -j 2 --message-format short "$@" 2>&1 | grep -E '^(error|warning)|^[^ ].*: (error|warning)|^error\[' | head -300
    return "${PIPESTATUS[0]}"
}
exec 9>"$lock"
echo "agent-check: waiting for the check lock" >&2
flock 9
if [ "$(cat "$CARGO_TARGET_DIR/.owner" 2>/dev/null)" != "$root" ]; then
    echo "agent-check: target last used by another checkout; refreshing source times of $root" >&2
    (cd "$root" && git ls-files -z -co --exclude-standard | xargs -0 -r touch -c --) && echo "$root" > "$CARGO_TARGET_DIR/.owner"
fi
rc=0
if [ $# -gt 0 ]; then
    run "$@" || rc=$?
else
    echo "== springboard lib + tests (no desktop-wayland)"
    run -p springboard --lib --tests || rc=$?
    echo "== shell bin (desktop-wayland)"
    run -p springboard --bin shell --features desktop-wayland || rc=$?
fi
[ $rc -eq 0 ] && echo "agent-check: ok" || echo "agent-check: FAILED (exit $rc)"
exit $rc
