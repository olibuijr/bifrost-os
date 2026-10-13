# BifrOSt codebase cleanup — 2026-10-08

> Status 2026-10-13 (shell 0.7.25): this is the record of the 2026-10-08 cleanup. Since then:
> `tools/tab.py` is gone, so use `tools/bifrost.py` (`bifrost` on the PATH) only. The `out/cleanup-*.log` evidence files
> and the `springboard-shell 0.4.30` release no longer exist in the tree. The workspace now has many more members
> (44 apps, 16 audio crates, `wm`, `gpu`, `layers`, `gfx2d`, `chrome`, `rfb`, `mcp`; see `springboard/Cargo.toml`), and
> the boot code lives in `springboard/bootweave`. The compositor is `bifrost-wm`. The skills are `bifrost`,
> `bifrost-dev`, `bifrost-boot` and `bifrost-audio`, under `~/.codex/skills`, plus the repo-level skills named in
> `AGENTS.md`. Open structural items listed at the end are tracked in `docs/handoff-backlog.md`.

The shared foundations now have explicit ownership:

- `springboard/core` (`bifrost-core`): JSON, SHA-256 and atomic persistence.
- `springboard/render` (`bifrost-render`): raster graphics, fonts, symbols, image decoding, appearance hooks, text caches and clipboard.
- `springboard/ui` (`bifrost-ui`): semantic controls, input events and scroll physics.
- `springboard/proto`: process protocol and descriptor transport.
- `springboard/sdk`: app event loop and rendering adapter. It consumes the foundations directly.
- `springboard/src/shell`: system UI and orchestration; app catalogue and tests are separate modules.
- `springboard/apps`: independently packaged applications. Notes exposes a library shared by both hosts.

The shell re-exports previous Rust paths to preserve app source compatibility. The SDK still depends on shell-specific list widgets and the audio client; removing those final edges requires separating Settings actions/chart accessories from generic lists and extracting service clients. Browser and other built-in app dependencies still live in the shell. No build-time or memory improvement is claimed without measurements.

`packaging/arch/runtime-packages.txt` is the shared runtime dependency source for both package routes and the installer. `package-files.sh` owns the shared payload, and both routes use the same install hooks. Source builds include packaging inputs; sibling Cargo dependencies are relative to the checkout.

Use `bifrost` for development, for example `bifrost pc publish` and `bifrost pc midget state`. Canonical entry point: `tools/bifrost.py`. `tools/tab.py` remains an executable/import compatibility shim for existing automation. `BIFROST_REPO`, `BIFROST_SERIAL`, `BIFROST_EMU`, and `BIFROST_SHOTS` are the new overrides; legacy `TAB_*` overrides remain fallback inputs. New captures go to `~/Pictures/bifrost`. The workspace path, installed `springboard-shell` package/service, data directories and protocol identifiers retain compatibility names so existing installations and recovery continue working.

Local skills are now `bifrost`, `bifrost-dev`, `bifrost-boot` and `bifrost-audio` under `~/.codex/skills`. Their discovery names and command references are updated.

Further structural work: remove the SDK's remaining shell dependencies; move installed-app catalogue snapshots to owned storage instead of leaked static allocations; split Settings navigation/actions from page rendering; isolate built-in browser/media/game dependencies with explicit feature boundaries; shorten HANDOFF into current state plus a historical archive. These changes should preserve macOS-style PC presentation and the tablet's iPad reference design.

## Release and verification

Published and installed on Midget: `springboard-shell 0.4.30.1791467826-1` through `bifrost pc publish --skip-build` and `bifrost pc midget update --restart`. Btrfs whole-system probation accepted the new boot; previous generation is 0.4.28.1791465805-1.

- Workspace acceptance: 2,799 tests passed, no failures, four ignored (including a documentation example).
- Focused desktop menu integration: one test passed with desktop presentation.
- Python tooling: all 150 tests passed; CLI compatibility help matches the canonical entry point.
- All four renamed skills validated; both package recipes load the shared dependencies, and the shared payload stages 15 files.
- Optimized incremental shell build: 1m44s, with Cargo timings and lld. Packaging/publishing: 14s. This is an extraction rebuild, not a benchmark against the old crate layout.
- Midget: actual reboot, exact installed version, recovery acceptance, unlock/input, Settings, App Store catalogue (no store error), and existing packaged Notes launch/render verified. No app port packages were published in this batch.

Evidence: `out/cleanup-acceptance.log`, `out/cleanup-desktop-test.log`, `out/cleanup-python-tests.log`, `out/cleanup-build.log`, `out/cleanup-publish.log`, `out/cleanup-install.log`; captures in `~/Pictures/bifrost/cleanup-{before,settings,store,notes,final}.png`.

During the multi-window Notes/App Store check, the shell logged roughly 31–35ms draw times. This was not measured with a matched pre-update scene, so it is not evidence of a cleanup-induced regression or a performance improvement. App rendering/composition remains a follow-up area.
