#!/bin/bash
# rea-page.sh DOC_URL: REA passive evidence of an official documentation page as Chromium renders it.
# Needs REA (~/.local/share/rea-tools/env.sh, see the rea-local skill) and a CDP browser: `rea-page.sh --start` runs a
# headless Chromium on 127.0.0.1:${PARITY_CDP_PORT:-9333}. Prints the REA evidence_id, title and rendered headings/links.
set -uo pipefail
PORT=${PARITY_CDP_PORT:-9333}
if [ "${1:-}" = "--start" ]; then
  prof=${XDG_CACHE_HOME:-$HOME/.cache}/parity-cdp-profile; mkdir -p "$prof"
  nohup chromium --headless=new --remote-debugging-address=127.0.0.1 --remote-debugging-port=$PORT \
    --user-data-dir="$prof" --no-first-run about:blank >/dev/null 2>&1 &
  for _ in $(seq 20); do curl -s http://127.0.0.1:$PORT/json/version >/dev/null && echo "CDP on 127.0.0.1:$PORT" && exit 0; sleep 0.5; done
  echo "Chromium did not start" >&2; exit 1
fi
[ -f ~/.local/share/rea-tools/env.sh ] && . ~/.local/share/rea-tools/env.sh
curl -s http://127.0.0.1:$PORT/json/version >/dev/null || { echo "no CDP browser on 127.0.0.1:$PORT (run: $0 --start)"; exit 1; }
id=$(curl -s -X PUT "http://127.0.0.1:$PORT/json/new?$1" | python3 -I -c "import json,sys;print(json.load(sys.stdin)['id'])")
sleep 6
(cd ~ && timeout 90 rea inspect-web-page http://127.0.0.1:$PORT "$id" --include-accessibility-text --observation-ms 800 --format json 2>&1) | python3 -I -c "
import json,sys
try: d=json.load(sys.stdin)
except Exception as e: print('REA output unreadable',e); sys.exit(0)
n=d.get('normalized_result') or {}
if not n: print('REA error:',json.dumps(d)[:300]); sys.exit(0)
print('rea evidence_id:',d.get('evidence_id')); print('title:',n['target']['title'],'url:',n['target']['url'])
names=[x['name'] for x in n.get('accessibility',{}).get('nodes',[]) if x.get('name') and x.get('role') in ('heading','link','StaticText','text')]
print('rendered:', ' | '.join(dict.fromkeys(names))[:3000])"
curl -s "http://127.0.0.1:$PORT/json/close/$id" >/dev/null
