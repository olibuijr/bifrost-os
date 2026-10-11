# Platform maturity: implementation record

Parent task: `task/001-mature-bifrost-platform.md` (milestones `task/002` to `task/011`).
Decided 2026-10-08: updates use HTTPS only; PC is the first target.

## Phase 0 status (002)

### Working tree ownership (snapshot 2026-10-08)

`git status --short` showed 67 entries at the time of this note.

- This task: `task/*`, `springboard/docs/platform-maturity.md`, `springboard/src/shell/update.rs`, `springboard/src/shell/diag.rs`, `springboard/src/deildu/http.rs`, `springboard/src/shell/store/net.rs`. `deildu/http.rs` and `store/net.rs` diffs contain only the `post_text` addition. `diag.rs` and `update.rs` diffs are this change; review them before committing.
- PC ports agent (per `HANDOFF.md`): `springboard/apps/*-desktop/` (14 `src/main.rs` type changes, manifests, changelogs), `springboard/Cargo.toml`, `springboard/Cargo.lock`, `springboard/CHANGELOG.md`, `packaging/arch/PKGBUILD`.
- Unclassified: `springboard/apps/{books,calculator,clock,podcasts,shortcuts,voicememos,notes-desktop}` source edits, `springboard/core/src/fsutil.rs`, `springboard/src/bare/app_policy.rs`, `springboard/src/shell/{apphost,desktop_bar,keyboard,mod}.rs`, untracked `tools/pc-app-smoke.py`.

The reviewing lead owns the unclassified group above: app review fixes, physical keyboard/notification integration, and atomic-write hardening. User authorized integrating and publishing the combined batch on 2026-10-08.

### Baselines

- Workspace: 2,799 tests passed, 4 ignored (`out/cleanup-acceptance.log`, from the cleanup batch).
- Draw time: 31–35 ms in a multi-window Notes/App Store scene, unmatched. Not a baseline.
- Scenes for 010 (to be measured): multiple visible windows, scrolling, resize, idle.

### HMAC triage and HTTPS-only updates

- `springboard/tools/update-hmac.key` is tracked in git. The shell no longer embeds it.
- Previously the shell accepted plain-HTTP manifests signed with an HMAC key that every client held, which gave no authenticity against a client that had the key. The LAN fallback `http://192.168.1.12:8190` is removed.
- Now: `shell/update.rs` accepts only `https://` manifest and binary URLs (manifest parse, `fetch_manifest`, `get`). `shell/diag.rs` diagnostics upload is HTTPS only, through `store::net::post_text` (new `deildu::http::Client::post_text`).
- Still open: `tools/bifrost.py` (around line 425) writes `latest.json.sig` with the same key, and the tracked key file remains. Remove both in a publisher change after the cut-over; no client reads them now.
- Behavior change: a diagnostics upload to a non-HTTPS update URL now fails with an explicit error.

### Path fixes

- Code comments referring to `docs/apps.md` and `docs/services.md` point at `springboard/docs/` (see `shell/apphost.rs`, `proto/src/lib.rs`, `sdk/src/lib.rs`, `shell/store/install.rs`, `src/svc.rs`). Not changed in this pass.

## Phase 1 finding (003): window identity is index-coupled

- `shell/desk.rs:331` names a window frame `apps()[(id-1)].name`, so the frame ID is the catalogue index plus one.
- `shell/desk.rs:357` maps a Wayland `app_id` back with `is.olibuijr.bifrost.{index+1}`.
- `bare/wayland.rs:156` sets the toplevel `app_id` to `is.olibuijr.bifrost.{frame.id}`.
- A catalogue reorder therefore changes window-to-app mapping. The fix must change the producer and consumer together (stable key on both sides); it is not a one-line change.

## Verification for this pass

- `cargo check --profile fast --bin shell`: no new warnings in the changed files.
- `cargo build --profile fast --bin shell`: built (`springboard/target/fast/shell`).
- `cargo test --profile fast --lib shell::update`: 7 passed (staging hash check, manifest HTTPS rules, parsing).
- `cargo test --profile fast --lib shell::diag`: 11 passed.
- Not run: full workspace tests, Midget acceptance. Not published.

## Integrated review

Only the HTTPS hardening and baseline investigation are implemented in this batch. Milestones 003–010 remain open; this is not completion of the platform maturity program. The review additionally rejects HTTP redirects for all store/update/diagnostic requests and preserves foreign temporary files on exclusive-create failure. PC pacman delivery retains its existing repository trust mechanism; this change concerns shell HTTPS transport. Legacy publisher signature output is retained for older installed clients during migration.
