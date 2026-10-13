#!/usr/bin/env bash
# Remove finished Claude Code worktrees under .claude/worktrees. A worktree goes only when it is clean (no changes
# to tracked or untracked files), not locked, and its branch is already merged into main, so no commit is lost.
# The branch ref stays. Each removal also deletes that worktree's build output (springboard/target*, target/).
# usage: tools/worktree-prune.sh [--apply]   (default: dry run, lists what would go)
set -euo pipefail
cd "$(dirname "$0")/.."
APPLY=0
[ "${1:-}" = "--apply" ] && APPLY=1
git worktree prune
git worktree list --porcelain | awk '/^worktree /{w=$2} /^locked/{l=1} /^$/{if(w!="") print w"\t"l; w=""; l=0}' |
while IFS=$'\t' read -r wt locked; do
  case "$wt" in "$PWD"/.claude/worktrees/*) ;; *) continue ;; esac
  [ "$locked" = 0 ] || { echo "skip (locked) $wt"; continue; }
  if [ -n "$(git -C "$wt" status --porcelain 2>/dev/null)" ]; then echo "skip (changes) $wt"; continue; fi
  b=$(git -C "$wt" symbolic-ref --short -q HEAD || true)
  if [ -z "$b" ] || ! git merge-base --is-ancestor "$(git -C "$wt" rev-parse HEAD)" main; then
    echo "skip (not merged) $wt ${b:-detached}"; continue
  fi
  if [ "$APPLY" = 1 ]; then
    git worktree remove "$wt" && echo "removed $wt"
  else
    echo "would remove $wt ($b)"
  fi
done
[ "$APPLY" = 1 ] && git worktree prune || echo "dry run: rerun with --apply to remove"
