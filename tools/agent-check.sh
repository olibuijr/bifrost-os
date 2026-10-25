#!/bin/bash
# The one compile check agents may run (owner, 2026-10-25: "allow cargo check"): `cargo check` only (no linking, no
# tests), in its own memory-limited systemd scope at the lowest CPU and I/O priority, one check at a time on Titan,
# with a shared target directory so dependencies are checked once for every worktree.
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
        cargo check --offline -j 2 --message-format short "$@" 2>&1 | grep -E '^(error|warning: unused)|^[^ ].*: error|^error\[' | head -60
    return "${PIPESTATUS[0]}"
}
exec 9>"$lock"
echo "agent-check: waiting for the check lock" >&2
flock 9
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
