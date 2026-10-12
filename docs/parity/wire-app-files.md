# Wire: Files app (springboard/apps/files)

Version 1.17.2 (system unchanged). Branch `wire/app-files`. Status: host-tested only, not yet run on Midget.

## What is wired now

| Matrix slug | Feature (docs/parity-matrix/data) | BifrOSt path | How a user reaches it | How to verify on Midget |
|---|---|---|---|---|
| file-coordination (Foundation, Files and Data Persistence) | "File coordination and presenters" (Partial) | `springboard/sdk/src/file_coordination.rs` (`NativeFileCoordinator::coordinate_writing`) via `springboard/apps/files/src/coordinate.rs` | File > Move to Trash on any item (also the context menu) | Put a lock holder on the item's folder (another coordinating process holds `coordinate_writing` on it) and trash an item in Files: after about 5 s the report reads "The item is in use by another app." and the item stays in place. With no holder, the item moves to Recently Deleted as before. |

Changed files: `springboard/apps/files/src/coordinate.rs` (new, model adapter and tests), `src/ops.rs` (`trash_now` wraps each discard), `src/main.rs` (`mod coordinate;`), `manifest.toml`, `Cargo.toml`, `CHANGELOG.md`, `springboard/Cargo.lock` (version line only).

## Considered and not wired (reasons)

- Find bar model (`springboard/ui/src/textfind.rs`, `TextFinder`): the model is for text in a text view. The Files search is a folder-tree query (`search.rs`, `FileManager`) with no text view and no "n of m" counter to drive. Wiring needs a status-bar counter design first.
- Tags and last-used (`springboard/ui/src/itemattrs.rs`, `Store`): the app already has tags (`springboard/src/files/tags.rs`, `TagIndex` sidecar, seven colours, Command-T). A second store would duplicate it; the attribute-read API is not needed.
- Thumbnails (`springboard/ui/src/thumbnail.rs` `Progressive`, `FileThumbnailRequest`; `springboard/sdk/src/peek.rs` `ThumbnailCache`): the cache only names PNG paths and the caller renders them. The app's Preview already decodes pictures itself. A real generator for PDF, text, audio or video does not exist, so there is nothing to show beyond the icon. Gap.
- File promises and typed drag payloads (`springboard/sdk/src/dnd.rs`, `springboard/proto/src/rich.rs`): typed drag exists in the SDK; the file-representation (sent/received transferred file) role is not modelled. Gap.
- File Provider action context (`springboard/ui/src/actionrequest.rs`, `fileaction.rs`): the Files app is the first-party browser; there is no extension hook to complete or cancel. Gap.
- Badges (`springboard/ui/src/filebadge.rs`): no SDK or Files API sets a per-item badge. Gap.

## Coordination limits

- The lock is advisory: a process that does not coordinate is not blocked.
- Locks are per thread (the trash job runs on a worker thread), so presenters in the Files process do not get own-write suppression for this path. The app registers no presenter, so nothing is lost today.
- Copy, move and paste (`transfer_now`) are not yet coordinated; they run inside the batch module and need a per-item wrap there.
- The lock waits up to `LOCK_TIMEOUT` (5 s) on the UI-visible job thread; a busy folder delays the job, not the window.

## REA evidence

- `ev_71f9582976e270c31d2e3e76a27165a7f2b01952f7e71456072e9f73a1d5272d`: official NSFileCoordinator page (developer.apple.com/documentation/foundation/nsfilecoordinator), "coordinates the reading and writing of files and directories among file presenters".
- `ev_2a5edb0e6a83a838b5b927b05c17d85cc40ed89f29631e4c26facfa59d5d8025`: official QLThumbnailGenerator page (developer.apple.com/documentation/quicklookthumbnailing/qlthumbnailgenerator), `generateBestRepresentation(for:completion:)` and `generateRepresentations(for:update:)`.

Official DocC read with `python3 -I tools/parity/docc.py foundation/nsfilecoordinator` and `quicklookthumbnailing/qlthumbnailgenerator` (the path omits the `documentation/` prefix).

## Remaining gaps (Files app)

- Find bar counter for filename search; typed file promises; a PDF/text thumbnail generator; per-item badges from apps; copy and paste coordination; presenter registration for the open folder.
- Tests: `springboard/apps/files/src/coordinate.rs` (3 unit tests). Not built in this round (no-build rule); the lead's integration build runs them.
