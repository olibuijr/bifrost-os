# Files polish (parity slice files-polish)

Scope: apps/files daily-driver behaviour. This round is a partial pass; most of the slice is unchanged.

## Done in this round

- Tabs (Files 1.25.0). Model: `apps/files/src/tabs.rs`, `TabStrip<T>` (new, insert after open, select, step with wrap,
  close keeping one tab, park). Tests: `tabs::tests`.
  - Keys: ⌘T new tab at the open folder, ⌘W close open tab, Ctrl+Tab / Ctrl+Shift+Tab step, Shift+T Tags (was T).
  - Not done: the tab strip is not painted, no menu bar item, no drag between tabs, no per-tab selection or view mode.
  - Needs Midget check: the keys once a strip is drawn.

## Not started in this round

- Finder reference (macos-ref ref27) not consulted: no Info.plist, menu or nib evidence recorded yet.
- Sidebar favourites and devices, Get Info, rename/duplicate/compress/trash, drag between windows, Go to Folder,
  Open With, new folder with selection, indexed search. Several already exist in code (see CHANGELOG); their
  gaps were not measured.
