# Drag and drop follow-up: drag image, background copies, Dock and Desktop drops (dnd-preview-async)

Round 75, slice `dnd-preview-async`. Builds on `docs/parity/dnd-source.md` (Files drags out) and `docs/parity/wire4-dnd-transfer.md`
(typed drag and drop). Code only; nothing was built or run in this round (the lead builds once). Every expected value in the
new tests was worked out by hand from the code.

## What is now wired

1. **A Files drag shows a drag image.** `springboard/apps/files/src/dragpreview.rs` (`drag_preview`) draws the icon of the first
   item (a folder or a page) with a count badge (red disc, white count, "99+" above 99) when the drag carries more than one item.
   It is drawn as RGBA on a 64 px canvas and sampled down by `dnd::preview_from_rgba` to 32 px. `dragout.rs` builds it when the
   drag starts (`drag_out_ask`) and gives it to the session (`drag_out_settle`). The shell already drew any valid preview at the
   pointer (`dnd_draw`); Files used to send an empty one.
2. **Files copies a drop from another app as a background job.** Before, `write_copy` ran on the UI thread in
   `drag_out_dest` (Perform) and in `drag_out_arrived` (a Dock icon's Open With). Now both call `Files::start_receive`, a new
   `JobKind::Receive` in `springboard/apps/files/src/ops.rs`. The job runs on the utility queue, streams each file in 256 KB chunks
   (`copy_chunks`), reports bytes every 100 ms through the same progress feed as Copy, and honours Cancel. A cancelled or failed
   copy removes its partial file. `start_job` and `start_receive` share `spawn_work` (one path for the job thread, the channel and
   the `Work` record). The drop is accepted when the job starts; a drop while another job runs is refused with the usual notice.
3. **Text and URL drops on a Dock icon open the app.** `springboard/src/shell/dnd.rs` (`dnd_zone_drop`): each `Payload::Url` goes to
   the app as `ToApp::Open { url }`, and each `Payload::Text` as `ToApp::Open { url: text_url(text) }`, the same way a share
   delivers them (`sharing.rs` `share_deliver`). Files still go through `inbound_files`. The drop was refused before when it had no
   file (`grants.is_empty()` returned 0).
4. **Folders dropped on the desktop are copied recursively, off the shell thread.** `dnd_copy_to_desktop` resolves each grant to its
   path under the account home, then runs `files::batch::transfer` (Copy, numbered names, recursive for folders) on a
   `spawn_named("desktop-drop")` thread. The shell no longer copies on its own thread, and folders are no longer skipped. The
   old `free_name` helper and its test were deleted: `batch` numbers taken names through `files::fs::unique_name`.

## How a user reaches it

- Drag image: select one or more items in Files and drag them. A single item shows its icon; several show the icon and a count.
- Background copy: drag files from another app onto a Files folder. The bar shows "Copying" with progress and can be cancelled.
  Open With on a file dragged in from a Dock icon uses the same job.
- Dock text or URL: drag selected text or a link (from an app that offers it as a drag item) onto a Dock icon. The app opens and
  receives the text or URL as an open request.
- Desktop folder: drag a folder from Files (or another app) onto the empty desktop. The copy appears in `Desktop` in the home
  folder once the thread finishes.

## How to verify on Midget (not run)

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Drag image: select three files in Files and start a drag. Expect a folder or page icon with a red badge showing `3` at the
   pointer. Select one item: no badge.
3. Background copy: copy a large file (over 1 GB) from another app onto a Files folder. Expect the Copying bar to move and the
   window to stay responsive; Cancel stops it and leaves no partial file.
4. Dock text: select a line of text in an app that drags it, and drop it on a Dock icon of another app. Expect that app to open
   and show the text.
5. Desktop folder: drag a folder with files in it onto the desktop. Expect a copy with the same name (numbered if taken) in
   `Desktop`, with the nested files.

## Tests added (host, not run)

- `dragpreview.rs`: canvas sampled to 32 px and valid; a single item has no badge; a folder uses the folder colour; several items
  draw a badge with a count; counts above 99 read "99+".
- `ops.rs` (`receive_tests`): a copy takes a free name and leaves nothing on failure; a cancelled copy removes the partial file;
  a batch copies each file in order; a cancelled batch stops before the next file.

## REA evidence

None. This round did not measure a reference bundle for these interactions; the behaviour follows the existing typed drag and drop
model and the shell's own Dock and desktop zones. Drag image and count badge are BifrOSt-specific choices.

## Gaps

- **Folders from another app onto a Files window** are not copied. A grant for a folder reaches Files as a directory handle; the
  streamed copy cannot recurse into it, and Files does not get a path (the shell keeps paths from apps). Desktop drops do copy
  folders, because the shell has the path.
- **No progress for desktop drops.** The desktop copy runs on its own thread with no bar and no notice on completion. A failed
  item is not reported to the user.
- **Drag image is a generic icon.** It is not the file's real thumbnail or the app icon; the shell draws the preview at the pointer
  with its alpha.
- **Text drops onto a Dock icon** are delivered as `ToApp::Open` with a data URL. An app that does not handle `Open` for text
  receives nothing. Text dropped on a window is still delivered as a typed drop item (unchanged).
- **Files' job is not queued.** A foreign drop while another job runs is refused with the notice; the dropped files are released.
- **Foreign file drops are copied, not moved**, as before.

## Matrix rows

Matrix data is not edited in this round (the lead publishes it). None of the entries below is closed by this slice; each advances:

| slug | feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| core-transferable | Drag and drop of typed items | springboard/src/shell/dnd.rs (`dnd_zone_drop`: Dock text and URL); springboard/apps/files/src/dragout.rs (drag image) |
| core-transferable | File transfer (file URL representations) | springboard/apps/files/src/ops.rs (`start_receive`, `receive_now`, `write_copy`); springboard/src/shell/dnd.rs (`dnd_copy_to_desktop`) |
| appkit-user-interactions | Drag and Drop | springboard/apps/files/src/dragpreview.rs (drag image) |

Expected status after Midget verification: `Drag and drop of typed items` can move to partial with these paths; `File transfer`
stays partial until folders reach Files windows and desktop drops report progress.
