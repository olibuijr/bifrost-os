#!/bin/bash
# Copy the pi agent setup (config, auth, extensions, skills, themes) from Titan to Midget's root account.
# Sessions, missions, run history and the web-search cache are not copied: they are history, not setup.
# Symlinks are dereferenced, because skill links point at paths that do not exist on Midget.
# The previous Midget copy is kept as ~/.pi/agent.prev.
# Usage: tools/copypi.sh [--dry-run]
set -euo pipefail
host=${MIDGET_HOST:-midget}
src="$HOME/.pi"
excludes=(
  --exclude=agent/sessions
  --exclude=agent/missions
  --exclude=agent/run-history.jsonl
  --exclude=agent/web-search-cache
  --exclude=.git
)

if [[ ${1:-} == --dry-run ]]; then
  tar -C "$src" -chf - "${excludes[@]}" agent | tar -tvf - | sed -n '1,40p'
  echo "... $(tar -C "$src" -chf - "${excludes[@]}" agent | tar -tf - | wc -l) entries would be copied to $host:/root/.pi/agent"
  exit 0
fi

ssh -o BatchMode=yes "$host" 'mkdir -p /root/.pi && rm -rf /root/.pi/agent.prev && { [ -e /root/.pi/agent ] && mv /root/.pi/agent /root/.pi/agent.prev || true; }'
tar -C "$src" -chf - "${excludes[@]}" agent | ssh -o BatchMode=yes "$host" 'tar -C /root/.pi --no-same-owner -xf -'
ssh -o BatchMode=yes "$host" 'chmod 600 /root/.pi/agent/auth.json /root/.pi/agent/models-store.json 2>/dev/null; chmod 700 /root/.pi/agent; chown -R root:root /root/.pi/agent; ls -la /root/.pi/agent'
echo "copied ~/.pi/agent to $host:/root/.pi/agent"
