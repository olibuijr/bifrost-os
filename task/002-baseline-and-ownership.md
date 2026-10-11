# 002 — Phase 0: baseline, ownership and security triage

Status: partially implemented; see `springboard/docs/platform-maturity.md`. Depends on: none. Parent: `001-mature-bifrost-platform.md`.

## Problem (verified)

- `springboard` has 38 dirty entries in `git status`: 24 modified and 14 type changes. The type changes are the `apps/*-desktop/src/` wrappers, which were symlinks at HEAD and are now a mix of symlinks and regular files.
- `HANDOFF.md` states another agent owns the PC app ports. The workspace member list (`springboard/Cargo.toml` around line 97) is a shared hotspot.
- `docs/codebase-cleanup.md` records a 31–35 ms draw time in a multi-window scene with no matched baseline.
- `springboard/tools/update-hmac.key` is embedded at `springboard/src/shell/update.rs:27` with `include_str!`. The source notes that clients can obtain it.
- The paths `docs/apps.md` and `docs/services.md` referenced by code are actually `springboard/docs/apps.md` and `springboard/docs/services.md`.

## Steps

1. Snapshot the working tree: `git status --short` and `git diff --stat` into `out/platform-baseline/`. Do not commit the snapshot.
2. Classify every dirty path as one of: this task, the PC ports agent, unrelated, or unknown. Record the table in `springboard/docs/platform-maturity.md` (create it). Mark paths this task may touch and paths it must not touch.
3. Record baselines without changing code: test count from the last acceptance log (`out/cleanup-acceptance.log`), frame-time and idle-CPU scene definitions for 010 (multi-window, scrolling, resize, idle), and the exact commands.
4. Security triage of the embedded HMAC key: check whether `springboard/tools/update-hmac.key` is tracked in git (`git ls-files`), which code paths accept HMAC-signed plain-HTTP manifests, and where the key is shipped. Do not print key material. Write findings and a recommendation into `platform-maturity.md`, then hand the decision to 011 and open decision 1 in `001`.
5. Fix path references: change `docs/apps.md` and `docs/services.md` in code comments to `springboard/docs/...` (`sdk/src/lib.rs`, `proto/src/lib.rs`, `shell/apphost.rs`, `shell/store/install.rs`, `src/svc.rs`), or point them at the repo-relative location.
6. Read the "Further structural work" list in `docs/codebase-cleanup.md` and tag each item as absorbed by 003, 004, or separate.

## Acceptance

- `platform-maturity.md` exists with the ownership table, baselines, scene definitions and HMAC triage.
- No code behavior changed. Only comments and documentation changed in this milestone, unless the path fix needed a trivial rename.
- The exit report lists the paths this task may touch.
