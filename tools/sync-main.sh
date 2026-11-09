#!/bin/bash
# Merge integration into main without touching the working tree (safe while a build or publish runs from this
# checkout), scan the commits GitHub has not seen yet, and push both branches to origin.
# Owner rule (2026-10-27): GitHub keeps the source history; push after a secrets and name scan. origin is PUBLIC.
#   tools/sync-main.sh            scan, merge, push
#   tools/sync-main.sh --dry-run  scan and show what would be merged and pushed
set -euo pipefail
cd "$(dirname "$0")/.."
dry=${1:-}
git fetch -q origin
for b in integration main; do
    echo "$b: $(git rev-list --count origin/$b..$b) commit(s) not on origin"
done

# secrets in the new commits, and the trademark name guard over the tree
if git rev-list --count origin/integration..integration | grep -qv '^0$'; then
    gitleaks git --log-opts="origin/integration..integration" --no-banner --redact . >/dev/null 2>&1 ||
        { echo "sync-main: gitleaks found something in origin/integration..integration; stopping (show it to the owner)" >&2; exit 1; }
fi
python3 tools/check-names.py >/dev/null || { echo "sync-main: tools/check-names.py failed; stopping" >&2; exit 1; }
echo "scans clean"

if git merge-base --is-ancestor integration main; then
    echo "main already contains integration"
    merged=$(git rev-parse main)
else
    tree=$(git merge-tree --write-tree main integration) ||
        { echo "sync-main: main and integration conflict; merge them by hand" >&2; exit 1; }
    if [[ $dry == --dry-run ]]; then
        echo "would merge $(git rev-list --count main..integration) commit(s) into main"
        exit 0
    fi
    merged=$(git commit-tree "$tree" -p main -p integration -m "merge: integration into main

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>")
    git update-ref refs/heads/main "$merged" main
    echo "main -> $(git log --oneline -1 main)"
fi
[[ $dry == --dry-run ]] && exit 0
git push -q origin integration main
echo "pushed integration $(git rev-parse --short integration) and main $(git rev-parse --short main)"
