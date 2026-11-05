#!/bin/bash
# Fast device check without a release (owner, 2026-10-25: per fix push, release once per batch): build the desktop
# shell (fast profile), strip it, install it over /usr/bin/springboard on Midget, restart the session (bootweave starts
# the window server and the shell again), unlock and screenshot.
# --wm also builds and pushes the window server and its chrome clients (menu bar, Dock, panels).
# --no-build pushes what is already built. The next `bifrost pc midget ota` restores the packaged binaries.
# Usage: BIFROST_LEAD_BUILD=1 tools/midget-push-shell.sh [--wm] [--no-build] [screenshot.png]
set -euo pipefail
here=$(dirname "$(readlink -f "$0")")
cd "$here/../springboard"
wm=0 build=1
while [[ ${1:-} == --* ]]; do
    case $1 in --wm) wm=1 ;; --no-build) build=0 ;; *) echo "unknown option $1" >&2; exit 2 ;; esac
    shift
done
out=${1:-/tmp/midget-push.png}
if (( build )); then
    "$here/lead-run.sh" cargo build --offline --profile fast --bin shell --features desktop-wayland
    (( wm )) && "$here/lead-run.sh" cargo build --offline --profile fast -p bifrost-wm --bin bifrost-wm -p bifrost-chrome --bins
fi
# target/fast name -> installed path on Midget
declare -A bins=([shell]=/usr/bin/springboard)
(( wm )) && bins+=([bifrost-wm]=/usr/bin/bifrost-wm [bifrost-menubar]=/usr/bin/bifrost-menubar [bifrost-dock]=/usr/bin/bifrost-dock [bifrost-panels]=/usr/bin/bifrost-panels)
K="$here/../out/lab/id_ed25519"
H=${MIDGET_HOST:-$(python3 -c "import sys;sys.path.insert(0,'$here');import midget;print(midget.HOST)")}
SSH=(ssh -i "$K" -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR "root@$H")
install=""
for b in "${!bins[@]}"; do
    strip -o "/tmp/push.$b" "target/fast/$b"
    # ssh + cat: Midget has no sftp server, which scp needs
    "${SSH[@]}" "cat > /tmp/push.$b" < "/tmp/push.$b"
    install+="install -m755 /tmp/push.$b ${bins[$b]} && "
    echo "pushed $b -> ${bins[$b]}"
done
"${SSH[@]}" "${install}systemctl restart springboard && sleep 6 && journalctl -u springboard -b --since '-10s' --no-pager | grep -iE 'panic|error' | tail -5" || true
python3 "$here/midget.py" unlock >/dev/null || echo "unlock failed; check the lock screen" >&2
sleep 2
python3 "$here/midget.py" shot "$out" --max 1280 | tail -2
python3 "$here/midget.py" state | grep -E '^(version|screen)='
