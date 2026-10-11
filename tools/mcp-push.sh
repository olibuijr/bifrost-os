#!/bin/bash
# Push a fast build of bifrost-mcp to Midget (/usr/local/bin) without a release.
set -euo pipefail
here=$(dirname "$(readlink -f "$0")")
cd "$here/../springboard"
[[ ${1:-} == --no-build ]] || cargo build --offline --profile fast -p bifrost-mcp
K="$here/../out/lab/id_ed25519"
H=${MIDGET_HOST:-$(python3 -c "import sys;sys.path.insert(0,'$here');import midget;print(midget.HOST)")}
strip -o /tmp/bifrost-mcp.push target/fast/bifrost-mcp
ssh -i "$K" -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR "root@$H" \
  'mkdir -p /usr/local/bin && cat > /usr/local/bin/bifrost-mcp.new && chmod 755 /usr/local/bin/bifrost-mcp.new && mv /usr/local/bin/bifrost-mcp.new /usr/local/bin/bifrost-mcp && bifrost-mcp --version' < /tmp/bifrost-mcp.push
