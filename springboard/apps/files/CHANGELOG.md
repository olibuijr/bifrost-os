## 1.18.0 — 2026-10-12

- Dark appearance: the content area behind the file list, grid and columns is the window's grey, not the pure black it was. It follows Light and Dark through the shared window colour.
- Get Info shows what an ISO or IMG disk image holds: the disc format, its volume name and size, and whether the file is complete or shorter than its volume ("Truncated").
- A folder that changes while it is still loading catches up: the changes are kept and applied once the listing is ready. If too many changes were dropped, the folder lists again.

## 1.17.2 — 2026-10-12

- Moving an item to the Trash takes the item's folder under the shared file coordination lock, so another app that coordinates its reads and writes never sees the item half moved. If the folder stays busy for five seconds the item is left in place and the report says "The item is in use by another app."
- Space shows the desktop's preview panel for the selection, and the arrow keys step through it; the selected row follows the item shown. Space again closes it. Without the panel the in-app preview still opens.
- Copy puts the selected files on the shared pasteboard, so another app can paste them. Paste takes the files another app copied, and the Copy or Cut from Files keeps its move.
- A document you open is indexed for desktop search by its name and folder.
- A long copy, move or duplicate shows its progress on the Files Dock icon until it finishes.

## 1.17.1 — 2026-10-12

- Tests: under the test pin the first folder is read at once, so a busy machine never snapshots "Loading…".
- View Options and the column headers use the named radius and type tokens (card radius 13 pt, callout and headline sizes).

## 1.17.0 — 2026-10-11

- Undo: the Edit menu names the step ("Undo Move", "Undo Rename", "Undo Trash", "Undo New Folder") and Undo is enabled only when there is something to undo (system 0.5.33).

## 1.16.0 — 2026-10-11

- Sidebar text is vibrant on the translucent sidebar (SDK `widgets::Vibrant`): labels, section headers and counts blend into the material (plus-lighter in Dark, plus-darker in Light), so the blurred windows and desktop behind tint them, as Finder's sidebar does. The sidebar is opaque while the window is inactive (system 0.5.25).

## 1.15.1 — 2026-10-11

- The open folder is watched without its subfolders (SDK `FSEventStreamFlags::SHALLOW`), so opening Computer or the home folder no longer walks the whole tree on the UI thread.

## 1.15.0 — 2026-10-11

- Undo (Edit > Undo, Cmd+Z) reverses the last move, rename, Move to Trash and New Folder, up to 20 steps. It never overwrites: a name taken again gets a number. A new folder is removed only while it is empty.
- Copies are written under a hidden temporary name and renamed when whole, so a cancelled or failed copy never leaves half a file. A move across file systems verifies the copy (kinds, names, sizes, links) before the original is deleted.
- A full disk stops the whole batch with "Stopped: the disk is full"; permission and vanished-file errors use Finder's wording. Broken symlinks now count as taken names, so "name 2" numbering and Rename see them.
- Removed `unwrap` calls from name numbering and New Folder; unit tests cover collision naming, temporary-name copies, verification, dangling links and Undo.

- Go menu: Go to Folder (Command-Shift-G) opens a path sheet pre-filled with the open folder; `~` expands to the home folder, Tab completes folder names (common prefix, trailing `/` when unique), Return opens, Escape cancels. Command-Shift-H Home, -D Desktop, -O Documents, -F Recents, Command-Option-L Downloads; Command-Up (Enclosing Folder) was already there.
- Spring-loaded folders: a drag held over a folder (or sidebar place) for 0.8 s opens it; the dragged items are kept across the navigation.
- Drag and drop: holding Option during the drag switches the drop from Move to Copy (the status bar says which).
- Quick Look shows JPEG and PNG pictures, and for other files the icon above kind, size, dates and tags.
- Recents, Get Info and Tags (seven colours in the sidebar, context menu and Command-T) were already in place.

- Finder views parity: a fourth view, as Gallery (Command-4): a large preview of the selected item with a thumbnail strip below it (Left and Right move through it). Command-1 to 4 switch Icons, List, Columns and Gallery.
- List view: sortable column headers (Name, Date Modified, Size, Kind) with a direction arrow; a second click on the same header reverses the sort. New Date Modified column. Folders have a disclosure triangle that opens them inline, indented.
- Columns view: drag a column separator to resize the columns; selecting a file shows a preview column (picture thumbnail or icon, kind, size, modified).
- Show View Options (Command-J, the toolbar sort button, or the background menu): icon size, text size, Group By (None, Kind, Date) and Sort By. Groups show a caption at the start of each group in the list view.
- Pictures (JPEG, PNG) show their thumbnail in the icon view, the gallery and the columns preview.
- Performance for big folders: the listing worker orders the rows (and renames the Trash's) on the dispatch queue, so the UI thread only swaps them in; only the rows or icon lines that reach the viewport are drawn; the row filter no longer allocates a lowercase name per row when the search is empty; icon names are cached per path and mtime in a bounded LRU (1024).
- Folder changes are folded into the open listing from the `FSEventStream` (create, delete, rename, modify of direct children) instead of listing again; a rescan flag or a flood of events falls back to a fresh listing. The loop wakes only while a batch is maturing.
- Micro benchmarks `bench_files_*` (sort, row window, label cache, filter at 50k entries), run by `tools/bench.py`.
- Agent control: the Files control socket serves `files.state`, `list`, `select`, `open`, `navigate`, `set_view`, `new_folder`, `rename`, `move`, `copy`, `trash`, `undo`, `search`, `get_info` and `tags`. Navigate, search, copy, move and trash answer when they finish (SDK pending replies). Edit > Undo reverts the last rename, move, copy or new folder made this way. MCP tool `files`, documented in `docs/bifrost-mcp.md`.

- Refactor, no behaviour change: `main.rs` is split into Finder-shaped modules (`window`, `sidebar`, `toolbar`, `views/{icon,list,column,gallery}`, `model`, `pathbar`, `statusbar`, `ops`, `search`, `info`, `quicklook`, `menus`, `keys`, `control`). `paint` and `event` now delegate to per-view and per-panel methods.

## 1.14.4

- Folder changes come from the SDK `FSEventStream` (`bifrost_app::fsevents`) instead of the app's own inotify code.

## 1.14.3

- Compliance gate: the app publishes its controls as the kit draws them. The search field stays open on a wide toolbar (640 pt and more) as Finder's does; on a narrow one it is still the round button.
- Background work runs on the dispatch pool (`DispatchQueue::global(.utility)`) instead of raw threads. Each job or search message wakes the loop, so the app has no 1-second idle timer.
- Show in Files: an inotify watch on the data folder wakes the loop when the shell leaves a request (no polling).
- The focused toolbar button's underline is drawn as a plain rect; the app has an icon (`icon.png`).

## 1.14.2

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.14.1 — 2026-10-10

- Sidebar and toolbar take the window's vibrancy (blurred wallpaper) on the GPU presenter.

## 1.14.0 — 2026-10-10

- Finder's sidebar: 160 pt wide, Recents first, then Favorites (Applications when there is such a folder, Desktop, Documents, Downloads), Locations (your home folder by name, Computer, Trash) and Tags with colour dots; a tag row lists the items with that colour. Recently Deleted is now called Trash.
- Icon view draws Golden Gate folders (two-tone blue with the back tab, embossed glyphs on Desktop, Documents, Downloads, Pictures, Music and Videos) at 64 pt, with 12 pt names and Finder's spacing.
- The status bar is hidden by default; show it with Command-/ or the background menu. When shown it is one row: the path and the item count.
- The round sidebar button by the traffic lights is gone; use Control-Command-S or the background menu.
- Search is a round magnifier button that opens into the field; Esc, or leaving it empty, closes it again.
- Larger toolbar buttons and sidebar glyphs (SDK: 36 pt capsules, 20 pt glyphs).

## 1.13.2 — 2026-10-10

- Files opens again on BifrOSt 0.4.90: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`. 1.13.1 exited at launch.

## 1.13.1 — 2026-10-10

- Clicking a toolbar button no longer leaves a blue focus underline under it; the focus ring shows only when you move to a button with Tab.

## 1.13.0 — 2026-10-10

- Golden Gate window (system 0.4.84): the sidebar runs to the top under the traffic lights and there is no separate title row. Drag the window by the empty part of the toolbar.
- The toolbar is Finder's: back and forward in one capsule, the folder name, the view modes as a segmented group (icons, list, columns), sort, new folder and select in one group, and a search field with a magnifier and a clear button.
- Sidebar rows have their own glyphs (house, desktop, document, download, pictures, music, videos, trash, clock, tag, computer); the open place is a grey plate with a bold name.
- Hide or show the sidebar with the sidebar button or Control-Command-S.
- The footer has a path bar: click a folder in it to go there.
- Icon view shows only the name under each item, as Finder does.

## 1.12.1 — 2026-10-10

- The search field follows Light and Dark: its plate, text and placeholder use the scheme colours instead of staying dark in light mode. Selected text in the field is white on the blue selection.

## 1.12.0 — 2026-10-10

- Follows Light and Dark from the system (system 0.4.83): the sidebar, toolbar, list and background use the scheme colours, and text on a selected row stays white on the blue selection.

## 1.11.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.
- Undo and Redo work in the search and naming fields.

## 1.11.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 21 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.11.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.

## 1.10.2 — 2026-10-08

- The footer says "1 item" for a single item.

## 1.10.1 — 2026-10-08

- The footer names Recents, Tags and Recently Deleted instead of their internal paths.

## 1.10.0 — 2026-10-08

- Columns view (Cmd+3, or the view button): the folders above the open one show as columns; click a folder to open it.
- Show in Files: other apps can open this app at a folder. A folder given as the first argument, or a request from the shell, opens there.
- A drag shows the destination: the sidebar place or folder under the finger is highlighted.

## 1.9.0 — 2026-10-08

- Colour dots show each item's tags, in the list and the icon view.
- Tags in Locations lists every tagged item.
- Drag to move: press an item and drag it onto a folder or a sidebar place. The footer names the target; release to move (as a background job).
- A folder given on the command line opens there.

## 1.8.0 — 2026-10-08

- Tags: Cmd or Ctrl+T or Tags… in the context menu opens a panel of the seven colours. A click turns a colour on or off for the whole selection.
- Tags are saved in the home folder and follow renames, moves and copies. Items sent to Recently Deleted lose their tags.

## 1.7.0 — 2026-10-08

- Paste asks when a name is taken: Keep Both, Replace or Skip. Keyboard: k, r, s; a capital letter applies the choice to the rest of the batch. Escape cancels. Duplicate still keeps both without asking.

## 1.6.0 — 2026-10-08

- Compress makes a .zip of the selection in the open folder; Extract unpacks a .zip into a new folder beside it.
- Both run as background jobs with progress, and Escape cancels them.

## 1.5.0 — 2026-10-08

- Get Info (Cmd+I or the context menu): kind, size, created and modified dates, location and tags. Folder totals stop counting after under a second.
- Escape or a click closes the panel.

## 1.4.0 — 2026-10-08

- Search looks through the folders under the open folder, not only the names in it. Hits stream in as they are found; Escape clears the search.
- The status line says how many items were found and where; clearing the search shows the folder again.

## 1.3.0 — 2026-10-08

- Recents in the sidebar: the newest files in Home, newest first. Nothing can be pasted or created inside Recents.
- Locations now lists Recents and Computer under one heading.

## 1.2.0 — 2026-10-08

- Select picks several items: the Select button turns taps into toggles, Cmd+A selects all, and the context menu acts on the whole selection.
- Copy, Cut, Paste, Duplicate and Move to Recently Deleted run on a background job with a progress line and bar. Escape cancels; the window stays responsive.
- A paste never overwrites: a name already in use is numbered ("name 2").
- Put Back returns a trashed item to the folder it came from; Home is used when that folder is gone.
- Status line shows the selection count and size.

## 1.1.0 — 2026-10-08

- Rename in place: Cmd+Return or Rename in the context menu. Enter saves, Escape cancels; a taken name is refused with a message.
- Copy, Cut and Paste files between folders (Cmd+C, Cmd+X, Cmd+V, Edit menu, context menu). Duplicate makes "name 2" beside the original.
- Move to Recently Deleted with Cmd+Backspace or the context menu. Recently Deleted is in the sidebar; Put Back returns an item to Home.
- New Folder with Cmd+Shift+N or on the empty background. Refresh stays in the background menu.
- Show Hidden Files with Cmd+Shift+. or the background menu.
- The sort button cycles Name, Date, Size and Kind. Cmd+1 and Cmd+2 switch icon and list view.
- Cmd+Up opens the enclosing folder; Cmd+[ and Cmd+] go back and forward; Cmd+F focuses search.
- Computer is a row under Locations. The footer shows the selected file's size, and failed actions say why.

## 1.0.5 — 2026-10-08

## 1.0.6 — 2026-10-08

- Rebuild against the reviewed shared PC app foundations.

- Use the SDK desktop context menu for Open, Quick Look, Copy Name and Refresh.

## 1.0.4 — 2026-10-08

- Adopt shared BifrOSt controls for search editing, selection, caret and keyboard focus.
- Support Cut/Copy/Paste/Select All through the desktop Edit menu and publish semantic toolbar controls.

## 1.0.3 — 2026-10-08

- Repaint only affected rows and the selection footer when selecting a file; leave unchanged controls and content intact.
- Avoid sending frames for input that does not change the visible UI.

# Files

## 1.0.2 — 2026-10-07

- Draw consistent toolbar symbols and avoid name collisions when creating folders.

## 1.0.1 — 2026-10-07

- Use SDK keyboard codes for search, navigation and opening files.

## 1.0.0 — 2026-10-07

- Split desktop Files into an independently updated App Store process.
- Add Finder-style sidebar, compact toolbar, icon/list views, breadcrumb path and item status.
- Browse the local account's folders with back/forward history, search, sorting, keyboard selection and Quick Look.
- Open documents through desktop file associations and create folders in the current directory.
