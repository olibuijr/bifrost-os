# 004 — Phase 2: remove the SDK and apps' dependency on the shell

Status: planned. Depends on: 003. Parent: `001`.

## Problem (verified)

- `springboard/sdk/Cargo.toml:12` depends on `springboard`. `sdk/src/lib.rs:25` re-exports `springboard::shell::list` and `sdk/src/lib.rs:41` re-exports `springboard::svc::audio`. `sdk/tests/apphost.rs:3-4` imports `springboard::gfx` and `springboard::shell::apphost`.
- Apps depend on the whole shell crate. `apps/notes/Cargo.toml:8` and `apps/notes-desktop/Cargo.toml:8` both list `springboard = { path = "../.." }`. Examples: `apps/weather/src/weather.rs:10-20` imports about ten shell modules (`layout`, `deildu`, `scene`, `shell::keyboard`, `shell::music::radio`, `shell::textfield`, `shell::weather`); `apps/games/src/games.rs:12-13` imports `shell::prefs` and `shell::Effect`; `apps/calculator/src/main.rs:68` uses `springboard::symbols`.
- `shell/list.rs` (about 1,100 lines) mixes reusable list logic with Settings actions, battery graphs, icons, and status rendering.
- `docs/codebase-cleanup.md` already names the remaining work: remove SDK shell dependencies, owned catalogue, split Settings navigation from page rendering, isolate browser/media/game dependencies.

## Steps

1. Build an import inventory: for each `apps/*` crate and the SDK, list every `springboard::` path used, and classify it as generic (move to `bifrost-ui`, `bifrost-render`, `bifrost-core` or a new shared crate), shell adapter (keep in shell), or feature-gated built-in. Save the table to `springboard/docs/platform-maturity.md`.
2. Extract the generic list model, layout, navigation state and input behavior from `shell/list.rs` into `bifrost-ui`. Put raster adapters in `bifrost-render`. Keep Settings action/page IDs and battery accessories in shell adapters.
3. Extract service clients from `springboard/src/svc.rs` (the `audio` client) into a shell-independent crate `bifrost-services`. Keep audio reconnection and wire behavior. Re-export the old `springboard::svc` path for source compatibility.
4. Move `pngenc`, `symbols`, `gfx` and the `shell::fsutil`/`prefs` helpers that apps use into shared crates, or re-export them through an SDK module, so SDK and apps stop naming `springboard`.
5. Make the SDK depend on `bifrost-ui`, `bifrost-render`, `bifrost-services` and `bifrost-proto`. Remove `springboard` from `sdk/Cargo.toml`. Keep `bifrost-proto` free of UI or shell dependencies.
6. Migrate one representative standalone app (choose `apps/calculator` or `apps/shortcuts`, the smallest) and the SDK test app to drop `springboard` entirely. Then migrate the rest by inventory order. Apps that need browser, media or game code keep a feature-gated shell dependency until step 7.
7. Isolate built-in browser, game and media dependencies behind explicit features or packages. Define which features ship on PC. Do not remove current built-in functionality.
8. Keep Settings page IDs and battery accessories in the shell. Do not mix them into shared lists.

## Coordination

The `apps/*-desktop` wrappers are owned by the PC ports agent (see 002). Do not change their `Cargo.toml` or `src` without that owner's agreement. Workspace members in `springboard/Cargo.toml` are shared; add new crates with minimal edits.

## Acceptance

- `cargo tree -p bifrost-app` and the representative app's `cargo tree` show no `springboard` dependency. Record the output in the exit report.
- Shell compatibility re-exports still compile for existing apps that are not yet migrated.
- An unrelated Settings source edit does not invalidate the standalone app's build graph (verify with Cargo's fingerprint or a rebuild that reports no recompilation of the app).
- Screenshot parity for extracted lists, compared against the macOS-style PC references.
- `springboard/docs/apps.md` and `springboard/docs/services.md` describe the new crates.
- Checks: focused builds of SDK, representative app, shell. No full workspace run here.
