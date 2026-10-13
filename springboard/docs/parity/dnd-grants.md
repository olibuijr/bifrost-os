# Drag and drop with file grants (slug dnd-grants)

Round 75. Scope from the owner: a drag that starts in Files (or any app) carries file grants to the drop target, drops onto
Dock icons and the desktop, spring-loaded folders across windows, and in Files an Empty Trash sheet, a New Tab button and
tab reorder by drag. This round lands the Files Empty Trash sheet and records the rest as open.

No parity-matrix row was closed by this round; the matrix is not edited here. Nothing in this file has been built or run
on Midget (the lead builds and tests).

## Evidence

- Shell drag state machine: `springboard/src/shell/dnd.rs` (`Session`, `State`, `dnd_msg`, `dnd_hand_over`, `map_files`).
- SDK protocol: `springboard/sdk/src/dnd.rs`, `springboard/sdk/src/transfer.rs` (`begin_drag`, `drag_item`).
- Read grants for a drop: `springboard/src/shell/docpanel.rs` `inbound_files` (broker `transfer`, `DocGranted` with
  `request == session`), used by `dnd_hand_over`, `sharing.rs` and `workspace.rs`.
- Files app: `springboard/apps/files/src/go.rs` (spring-loaded folders inside one window), `main.rs` (in-app drag to a folder
  or sidebar place), `tabs.rs` (tab model), `menus.rs`, `ops.rs`.
- Reference: the round-75 reference extracts for this slice were not checked in this run, so the behaviour comes from the
  existing code. Re-check the Files and drag rows against `~/Projects/macos-ref/extract/ref27/` before closing them.

## Rows

### Cross-app drag with file grants (sdk `dnd`, shell `dnd.rs`)

- API: `NSDraggingSource` (`beginDraggingSession`), `NSDraggingDestination` (`registerForDraggedTypes`, `draggingEntered:`,
  `performDragOperation:`), `NSPasteboard` file items.
- BifrOSt path: `sdk/src/dnd.rs`, `src/shell/dnd.rs`, `src/shell/docpanel.rs` `inbound_files`.
- Done: destination side works end to end in the shell state machine (register, Entered, Updated, Exited, Perform with
  fresh read grants made for the destination app, Ended). Unit tests cover negotiation, hit-testing and grant remapping.
- Still open: the source side never begins a drag with file items. `Payload::File` needs a grant that the source app holds,
  and today grants only come from user-picked panels (`DocOpen` and `DocSave`). Files holds the `storage` permission, not
  `documents`, so it has no grant for the paths it lists. The fix is a protocol decision: a shell-issued grant for the
  paths of a drag the user started on a Files window (the shell can check the source is the system Files app and the
  paths are under home), or a `ToShell` request that makes a grant for a dragged path. Not done this round.
- Midget check needed once the source exists: drag an image from Files onto a window that registered `public.image`.

### Drop onto Dock icons and the desktop

- Status: not started. `dnd_target` only hit-tests app windows (`desk.pick` with `Hit::Content`). A Dock icon of a running
  app would need a target that maps to its slot; a non-running app needs launch-then-deliver, which the shell does not do
  for drags yet. The desktop as a target has no registration path.
- Midget check needed.

### Spring-loaded folders across windows

- Status: inside Files only (`go.rs` `spring_hover` and `spring_tick`, 800 ms). A drag from another window does not reach
  Files as a destination yet (Files does not call `dnd::register_types`), so a folder across windows is not possible until
  the Files destination side exists.

### Files: Empty Trash confirmation sheet

- API: `NSAlert` with a destructive button, sheet over the window.
- BifrOSt path: `springboard/apps/files/src/empty_trash.rs` (sheet, hit-testing, input), `ops.rs` `JobKind::EmptyTrash` and
  `delete_now` (the worker job), `menus.rs` `MENU_EMPTY_TRASH` (background menu in Recently Deleted), `window.rs` paint.
- Done: the menu item appears only in Recently Deleted. The sheet asks first, Return or the red Empty Trash button deletes
  every item for good on the worker (progress shown by the job bar), Escape or Cancel closes it. The `.origins` bookkeeping
  file is skipped and pruned after the job. Unit tests cover the sheet frame, the button layout and touch hit-testing.
- Still open: Midget check of the sheet in Dark and Light (the destructive fill and the white label). Undo entries for earlier Move to Trash steps stay in the undo list (their restore fails cleanly when the
  item is gone).

### Files: New Tab button and tab reorder by drag

- Status: not done. The tab model (`tabs.rs`) and ⌘T, ⌘W, Ctrl+Tab exist, but the tab strip is not drawn, so there is no
  button to press and no tab to drag. Drawing the strip changes the window's vertical layout, so it needs a Midget check
  before reorder can be added on top. `TabStrip::move` is the next model step.

## Shared-file edits

None. Edits are inside `springboard/apps/files/` (`src/main.rs` adds `mod empty_trash;` and one routing line; `src/window.rs`
adds one paint call; `src/go.rs` adds one field; `src/menus.rs` adds one menu id and one action; `src/ops.rs` adds one job
kind and one worker function). `springboard/apps/files/Cargo.toml`, `manifest.toml`, `CHANGELOG.md` and `Cargo.lock`
(the `bifrost-files` version line) are bumped to 1.26.0.

## Migrations and deletions

None.
