#!/bin/bash
# Fast device check without a release: build the desktop shell (fast profile), strip it, install it
# over /usr/bin/springboard on Midget, restart the session, unlock and screenshot.
# The next `bifrost pc midget update` restores the packaged binary.
# Usage: tools/midget-push-shell.sh [--no-build] [screenshot.png]
set -euo pipefail
here=$(dirname "$(readlink -f "$0")")
cd "$here/../springboard"
[[ ${1:-} == --no-build ]] && shift || cargo build --offline --profile fast --bin shell --features desktop-wayland
out=${1:-/tmp/midget-push.png}
strip -o /tmp/springboard.push target/fast/shell
K="$here/../out/lab/id_ed25519"
H=${MIDGET_HOST:-$(python3 -c "import sys;sys.path.insert(0,'$here');import midget;print(midget.HOST)")}
# ssh + cat: Midget has no sftp server, which scp needs
ssh -i "$K" -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR "root@$H" "cat > /tmp/springboard.new" < /tmp/springboard.push
bifrost pc midget ssh "install -m755 /tmp/springboard.new /usr/bin/springboard && systemctl restart springboard && sleep 5 && journalctl -u springboard -b --since '-8s' --no-pager | grep -iE 'gpu|panic|error' | tail -5"
bifrost pc midget key enter >/dev/null
sleep 2
shoot midget "$out"
