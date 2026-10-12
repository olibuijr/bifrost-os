#!/bin/bash
# LEAD ONLY. The lead's one integration build; agents never run it (they only code and commit, see
# docs/bifrost-pc-workflow.md "Agent waves"). The hook ~/.claude/hooks/no-agent-builds.sh blocks cargo for agents and lets
# a command through when it is prefixed BIFROST_LEAD_BUILD=1:
#
#   BIFROST_LEAD_BUILD=1 tools/lead-check.sh
#
# Runs `cargo check --offline` over the whole workspace (all targets: `cargo test` also builds examples that
# `--tests` skips) and over springboard + bifrost-wm with the desktop-wayland feature, then prints errors and warnings by
# file and writes out/lab/warn_groups.json: the warnings split into at most 8 file-disjoint groups (no file in two groups,
# so a Haiku warnings wave can edit in parallel without merge conflicts).
set -u
cd "$(dirname "$0")/.."
ROOT=$PWD
mkdir -p out/lab
LOG=$ROOT/out/lab/lead-check.log
: > "$LOG"
# the cargo workspace is springboard/; file paths below are relative to it
echo "== cargo check --workspace --all-targets"
(cd springboard && cargo check --offline --workspace --all-targets --keep-going -j 6 --message-format=short 2>&1 | tee -a "$LOG" | grep -E '^[^ ]+:[0-9]+:[0-9]+: (error|warning)' | tail -n 5)
echo "== cargo check -p springboard -p bifrost-wm --features springboard/desktop-wayland --tests"
(cd springboard && cargo check --offline -p springboard -p bifrost-wm --features springboard/desktop-wayland --tests --keep-going -j 6 --message-format=short 2>&1 | tee -a "$LOG" | grep -E '^[^ ]+:[0-9]+:[0-9]+: (error|warning)' | tail -n 5)
echo "== bifrost-update-guard tests (packaging/arch/recovery/guard.rs: plain rustc outside the workspace, shipped by lab-push and build-iso)"
mkdir -p out/lab/guard-test && rustc --edition 2021 --test packaging/arch/recovery/guard.rs -o out/lab/guard-test/t 2>&1 | tee -a "$LOG" | grep -E '^(error|warning)' | head -n 5
[ -x out/lab/guard-test/t ] && out/lab/guard-test/t -q 2>&1 | tail -n 2
echo "== bootweave-efi (x86_64-unknown-uefi; excluded from the workspace, so it is built here)"
cargo build --offline --manifest-path springboard/bootweave/efi/Cargo.toml --target x86_64-unknown-uefi --profile release --message-format=short 2>&1 | tee -a "$LOG" | grep -E '^[^ ]+:[0-9]+:[0-9]+: (error|warning)' | tail -n 5

python3 - "$LOG" "$ROOT/out/lab/warn_groups.json" <<'EOF'
import collections, json, re, sys
log, out = sys.argv[1:3]
pat = re.compile(r'^(?P<file>[^\s:]+\.rs):(?P<line>\d+):\d+: (?P<kind>error|warning)(?:\[[^\]]*\])?: (?P<msg>.*)$')
seen, errors, warnings = set(), collections.defaultdict(list), collections.defaultdict(list)
for raw in open(log, errors="replace"):
    m = pat.match(raw.strip())
    if not m:
        continue
    key = (m["file"], m["line"], m["msg"])
    if key in seen:  # the two runs report shared crates twice
        continue
    seen.add(key)
    (errors if m["kind"] == "error" else warnings)[m["file"]].append(f'{m["line"]}: {m["msg"]}')
print(f"\n== errors: {sum(map(len, errors.values()))} in {len(errors)} files")
for f, rows in sorted(errors.items(), key=lambda kv: -len(kv[1])):
    print(f"  {len(rows):4} {f}")
print(f"== warnings: {sum(map(len, warnings.values()))} in {len(warnings)} files")
for f, rows in sorted(warnings.items(), key=lambda kv: -len(kv[1]))[:25]:
    print(f"  {len(rows):4} {f}")
# file-disjoint groups: files of one directory stay together while it fits, then greedy fill of the lightest group
by_dir = collections.defaultdict(list)
for f in warnings:
    by_dir[f.rsplit("/", 1)[0]].append(f)
groups = [{"files": [], "count": 0} for _ in range(min(8, max(1, len(warnings))))]
for d, files in sorted(by_dir.items(), key=lambda kv: -sum(len(warnings[f]) for f in kv[1])):
    for f in sorted(files, key=lambda f: -len(warnings[f])):
        g = min(groups, key=lambda g: g["count"])
        g["files"].append(f)
        g["count"] += len(warnings[f])
doc = [{"group": i + 1, "warnings": g["count"], "files": {f: warnings[f] for f in sorted(g["files"])}}
       for i, g in enumerate(groups) if g["files"]]
json.dump(doc, open(out, "w"), indent=1)
print(f"== {len(doc)} warning groups -> {out}")
EOF
