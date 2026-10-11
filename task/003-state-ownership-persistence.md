# 003 — Phase 1: stable ownership and persistence

Status: planned. Depends on: 002. Parent: `001`.

## Problem (verified)

- `springboard/src/shell/apps.rs:64-71` rebuilds the catalogue with `Box::leak` on every rebuild and returns `&'static [AppDef]`. The cache only grows, capped at 200 installed apps.
- Installed apps are identified by positional `AppId::External(u8)` (`shell/apps.rs`). The index is used across about 15 files, including `shell/desk.rs`, `spotlight.rs`, `hwkeys.rs`, `switcher.rs`, `desktop_bar.rs`, `notify/lockwidgets.rs`, `splitview.rs`, and `notify/mod.rs`.
- `shell/desk.rs:357` builds a window identifier from the list index (`is.olibuijr.bifrost.{index+1}`). A catalogue reorder therefore changes an app's identity. This is a bug.
- `springboard/core/src/fsutil.rs:14-28` writes temporary files named `.write-{pid}-{serial}.tmp` with `File::create` (truncating, not exclusive). It ignores directory-sync errors (`let _ = ...`). The shell re-exports a copy as `shell::prefs::write_atomic` and `shell::fsutil::write_atomic`, used by `apps/calculator/src/history.rs`, `apps/freeform/src/freeform.rs`, and `apps/shortcuts/src/model.rs`.
- `shell/apphost.rs` already uses owned `Arc` installed snapshots; build on that.
- `Box::leak` in `shell/notify/mod.rs:1251-1253` is inside the test module (starts line 885) and is not runtime. `widgets/terminal.rs:54`, `termtouch.rs:176`, and `today.rs:750` must be classified as once-only or repeated before changing.

## Steps

1. Replace leaked catalogue storage with an owned immutable snapshot (`Arc<[AppDef]>` or equivalent). Give `AppDef` owned `String` fields. Keep the `apps()` call sites working through the snapshot.
2. Introduce a durable app key: the manifest app ID. Use generation-aware runtime handles (`slot` plus generation) for live references. Persisted home and dock configuration references the manifest ID, never the index.
3. Migrate existing home and dock configuration once: map old positional entries to manifest IDs, keep the built-in Files and Browser replacement behavior, keep a backup of the old file, and make the migration idempotent.
4. Define removal: a running window keeps its launch-time package snapshot and is never rebound to a different app. Reinstall with the same manifest ID is a new generation. Uninstall with a running window closes the window or marks it orphaned.
5. Fix the window identifier at `shell/desk.rs:357` to use the manifest ID.
6. Inventory `&'static` assumptions and positional IDs; migrate persistence, dock/focus, notifications, menus, and launch in one pass.
7. Consolidate to a single atomic writer in `bifrost-core`. Use exclusive temporary creation (`create_new` with a random name), deliberate permissions, cleanup on failure, and propagated directory-sync errors for strict writes. Keep a best-effort mode for preference files. Document unsupported filesystems and concurrency limits instead of promising more.
8. Replace the shell and app re-exports with the core writer; keep the old paths as compatibility re-exports.

## Acceptance

- Repeated catalogue refresh frees obsolete metadata once readers drop it (test with a counter or weak reference).
- Rename, removal and reordering never change an app's identity or rebind a running window.
- Existing home configuration migrates, and a second run is a no-op.
- Concurrent writers never leave a partial file; strict persistence reports a directory-sync failure.
- Failure tests: exclusive-create collision, rename failure, permission denial, and a migration from a corrupt file. Not only happy-path tests.
- Checks: focused tests for `bifrost-core` and the shell catalogue; full workspace run belongs to the batch acceptance.

## Compatibility

Persisted configuration gains manifest-ID references. Old files still load through the migration. Installed apps see no protocol change.
