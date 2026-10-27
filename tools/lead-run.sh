#!/bin/bash
# Run a heavy lead command (a cargo build or test, bifrost pc publish, os-verify) in its own memory-limited systemd
# scope at low CPU and I/O priority. Titan has 15 GB; a springboard build can reach ~9 GB, and in the terminal's scope
# that made systemd-oomd kill the whole terminal (2026-10-22). In its own scope the build reclaims memory and slows
# down past MemoryHigh, and at worst only the build is killed at MemoryMax.
# Disk hygiene (2026-10, the target dir reached 1.3 TB and filled /home): a loud warning before and after the command
# when /home is over LEAD_DISK_WARN percent (default 85), and tools/target-prune.sh after it (stale incremental
# sessions, the unused debug profile). The command's exit status is returned.
# Usage: BIFROST_LEAD_BUILD=1 tools/lead-run.sh COMMAND [ARGS...]
#        LEAD_MEM_HIGH=7G LEAD_MEM_MAX=9G (defaults) to change the limits.
set -uo pipefail
[[ $# -gt 0 ]] || { echo "usage: tools/lead-run.sh COMMAND [ARGS...]" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd)
disk_alert() {
    local used
    used=$(df --output=pcent /home 2>/dev/null | tail -n1 | tr -dc '0-9')
    if [[ -n $used && $used -ge ${LEAD_DISK_WARN:-85} ]]; then
        printf '\n!!!!!!!! DISK ALERT: /home is %s%% full (limit %s%%). Free space now: tools/worktree-prune.sh --apply, tools/target-prune.sh, old out/ and backups. !!!!!!!!\n\n' \
            "$used" "${LEAD_DISK_WARN:-85}" >&2
    fi
}
disk_alert
systemd-run --user --scope --quiet --collect --unit="lead-build-$$" \
    -p MemoryHigh="${LEAD_MEM_HIGH:-7G}" -p MemoryMax="${LEAD_MEM_MAX:-9G}" \
    nice -n 19 ionice -c3 "$@"
rc=$?
nice -n 19 ionice -c3 "$here/target-prune.sh" || true
disk_alert
exit $rc
