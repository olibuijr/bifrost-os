# Drag and drop between windows and apps (slice dnd-source)

Round 75. Continues the typed drag and drop of wave 5 (`sdk/src/dnd.rs`, `src/shell/dnd.rs`), which could not carry files
dragged out of Files: a file payload needs a read grant, and no app had one for a folder listing.

## Reference evidence

No reference file was measured for this slice. The behaviour follows the existing typed drag protocol in this repo and the
Files behaviour already in `apps/files` (drag inside Files, spring-loaded folders, Option copies). The reference index
(`ref27-index.tsv`) was not consulted for it; the drop-zone choices (Dock opens the files, desktop copies them) are BifrOSt
decisions, to be checked against the reference when the lead extracts the Dock and Files reference resources.

## Rows and BifrOSt paths

| Row (matrix data file) | API | BifrOSt path | Status |
|---|---|---|---|
| Drag and Drop (appkit-user-interactions.json, appkit-user-interface.json) | NSDraggingSource, NSDraggingDestination, NSDraggingSession, NSPasteboard drag | `sdk/src/dnd.rs` (`begin`, `register_types`, `reply`, `performed`, `cancel`), `src/shell/dnd.rs`, `apps/files/src/dragout.rs` | file items between Files and other apps; Dock and desktop drops (below) |
| Drag and drop of typed items (core-transferable.json) | draggable / dropDestination on typed payloads | `sdk/src/transfer.rs` (`drag_item`, `drop_values`) unchanged; file payloads now carry grants | file items usable through the typed API |
| Drag and drop (swiftui-events-a11y-integration.json) | imperative drag and drop | same as above | as above |
| Image data in drag and drop (image-i-o.json) | image payloads in a drop | preview and file items; the drag preview is still empty for Files | the drop of an image file works; the drag image of Files does not |

Matrix rows are not edited here (the lead closes them). Candidates for closure: the rows above whose status is `partial`.

## What is done

- Read grant for a dragged path (`proto` `ToShell::DocGrantPath { request, path }`, tag 0x11b; `sdk` `Ctx::grant_path`;
  `src/shell/docpanel.rs` `Broker::drag_grant`). The app must hold the `documents` permission. The path must lie under the
  account's home, and symlinks and folders outside are refused (`Denied`). An app with only `documents` can be granted just
  paths it already holds a grant for (open or save panel, drop, share); an app with `storage` (Files) already has the home
  bound read-write, so it is not held to that list. The reply is `DocGranted` with the descriptor, as for
  an open panel. The grant is recorded for the app and the shell releases it when the drag ends.
- Files begins a session with its items: on a drag past the slop it asks for a grant per item (`Files::drag_out_ask`); once all
  are granted it calls `dnd::begin` with `Payload::File` items (`drag_out_settle`). Until then the drag is the in-window one.
- Source grants are released when the session ends (`Shell::dnd_release_sources`, on `DragEnded`, on the source going away).
  Files also releases the grants it holds when its session ends (`drag_out_ended`), so a refused begin leaves nothing.
- Destination grants: an app that takes the files gets a read grant of its own, as before (`Broker::transfer`). A window of
  the source app itself gets the source grants as they are (no transfer; `dnd_hand_over`).
- Files as a drop target: the window registers `public.data` (every file), reports the folder under the pointer, and replies
  Copy (from another app) or Move, Option-Copy inside Files (`drag_out_dest`). Files from another app are held until the drop is
  performed, then copied into the folder under the pointer with a free name (`write_copy`). Spring-loaded folders open the same way
  as in-window drags. The status bar says "Copy N items to <folder>" for a drop from another app.
- Dock: a drop on a Dock icon of an app opens the files in it (`dnd_zone_drop`, `Zone::Dock`), through the same route as Open With
  (`desk_open_or_start` and `inbound_files`). Files' own icon maps to the installed Files app. Files receiving files with no drop
  waiting (a Dock drop) copies them into the folder shown (`drag_out_arrived`).
- Desktop: a drop on the desktop (no window under the pointer, below the menu bar) copies the files into `Desktop` in the account's
  home, made if missing; a taken name gets a number (`free_name`, `dnd_copy_to_desktop`). Only copies: the source is not moved.

## What is not done

- Folders can be dragged between windows of Files only as grants (`open_any`), and a receiving app gets them as a
  descriptor; the copy paths (Files and the desktop) copy regular files only and skip folders.
- The drag image of Files is empty (`DragPreview::default()`): the shell draws no image for a Files drag.
- A drop that the shell refuses at begin (another session running, Files not in front) ends the drag at once; the in-window
  drag is then gone too. Files does not restart it.
- A drop on Files from another app copies synchronously on the UI thread; large files will hitch.
- Dropping a file onto Files' own Dock icon copies it into the folder shown (the Open With route has no path to open the file
  in place).
- Text and URL items dropped on the Dock or the desktop are ignored (only file items take those zones).

## Needs Midget check

- A Files item dragged over another app's window: the hover highlight and the status line while the pointer is over another window.
- Dragging a file from Files onto the Dock icon of another app, and onto the desktop (copy appears in `~/Desktop`).
- Two Files windows: dropping a folder-item from one onto the other, and the spring-loaded folder that opens across windows.
- A drag that starts before its grants come back, released quickly (the drag must stay in-window and not stall).
