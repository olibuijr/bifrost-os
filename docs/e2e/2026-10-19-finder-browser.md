# E2E on Midget: Files (Finder) and Browser, 2026-10-19

Midget 0.10.5 (`integration` at `daa0bcf77`), 2560x1440, Light appearance. Computer use through
`tools/midget.py shot/click/type/key/scroll`, about 10 minutes (21:21 to 21:31). Reference images:
`goldengate_refimages/27-Golden-Gate-Finder-*.png` and `27-Golden-Gate-Safari-*.png`.

Evidence images are next to this file in `docs/e2e/2026-10-19-finder-browser/`. They are on disk only
(`*.png` is gitignored), so read them from the main checkout: `/home/olibuijr/Projects/bifrost-os/docs/e2e/2026-10-19-finder-browser/`.

## What passed

- Files: icon, list and column views; double-click opens a folder; Back; sidebar navigation; New Folder;
  Move to Trash (Cmd+Backspace) and the Trash listing; Rename from the context menu; the search field filters, and
  Esc clears it (the round 82 Esc key-code problem no longer shows here).
- Browser: launch from the Dock; Cmd+L selects the URL and shows history suggestions; page loads (example.com,
  bifrost.olibuijr.com); Arabic right-to-left text; link clicks; Back; tab switching; closing a tab with its X;
  wheel scrolling; the "..." menu (separators and shortcuts are correct); the History menu; Cmd+F find with a match
  count; clicking the Dock icon brings back a hidden window with its tabs.

## Findings

Severity: **P1** breaks a basic task or readability, **P2** wrong behaviour or a clear visual difference from the
reference, **P3** polish or parity gap. Each finding is one fix brief.

### Files

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| F1 | P2 | Toolbar New Folder creates "New Folder" but does not select it or start rename; typing does nothing | New item selected, name in edit mode with the name selected | (screens s02, s03) |
| F2 | P2 | Return on a selected item opens it | Return starts rename; Cmd+O and Cmd+Down open, Cmd+Up goes to the enclosing folder | |
| F3 | P2 | Rename field does not select the name, so typing appends ("New FolderReports"); the item is deselected after commit | Base name selected (without extension); Return commits, Esc cancels; item stays selected | `F3-rename-appends.png` |
| F4 | P1 | Item context menu opens about 60 pt above the pointer over the toolbar; the surface is far taller than its items (blank area over the Dock); no separators; opaque, square, no shadow | Top-left at the pointer, height fits the items, grouped with separators, menu material, shortcuts right-aligned. The Browser "..." menu already renders correctly | `F4-context-menu.png`, ref `27-Golden-Gate-Finder-Menu.png` |
| F5 | P2 | Column view: the selected row in a column without focus draws black text on blue, and its blue folder icon vanishes | Focused column: accent plate with white text; other columns on the path: inactive grey plate | `F5-list-and-column-view.png` (right) |
| F6 | P2 | Search only filters the current folder; no scope bar; title unchanged | Title "Searching ...", scope bar (whole computer, current folder), recursive results | `F6-search-no-scope.png`, ref `27-Golden-Gate-Finder-Search.png` |
| F7 | P2 | Against the Home reference: grey toolbar band (the reference shares the content background with floating button groups); icons about half the reference size (icon width / sidebar row pitch 1.9 vs 3.7); folder glyphs nearly invisible; Locations icons blue (reference: grey); no Applications, Network; toolbar lacks Group, Share, Tags, action menu | Match the reference | `F7-finder-home-vs-ref.png` (left reference, right ours), ref `27-Golden-Gate-Finder-Home.png` |

### Browser

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| B1 | P1 | Cmd+W hides the whole window (7 tabs); the tabs come back on a Dock click. The "..." menu lists Close Tab Cmd+W | Cmd+W closes the active tab; the last tab closes the window | `B1-after-cmd-w.png`, `B1-dock-reopen-tabs-kept.png` |
| B2 | P2 | Menu bar order: app, Develop, History, File, Edit, View, Window, Help; no Bookmarks menu | App, File, Edit, View, History, Bookmarks, Develop, Window, Help | `B2-more-and-history-menus.png` |
| B3 | P2 | New Tab is a blank white page | Start Page: Favorites, Frequently Visited, Recently Viewed (no third-party lookups) | `B3-start-page-vs-ref.png`, ref `27-Golden-Gate-Safari-Start-Page.png` |
| B4 | P2 | U+3002 (ideographic full stop) renders as a missing-glyph box on example.com; U+FF0C renders | CJK punctuation covered by the fallback font | `B4-cjk-tofu.png` |
| B5 | P3 | Back returns to the top of the page | Back and Forward restore the scroll offset | |
| B6 | P3 | Reopen Closed Tab stays disabled after closing a tab with its X | Closed tabs can be reopened (Cmd+Shift+T) | `B2-more-and-history-menus.png` |
| B7 | P2 | Toolbar against the reference: reload, + and ... are large white circles; star sits outside the field; reload outside the field; no sidebar toggle, Share or tab overview; no page theme tint | Glass button groups at toolbar size; page menu icon and reload inside the field; Share, +, tab overview | `B7-toolbar-vs-ref.png`, ref `27-Golden-Gate-Safari-v27.png` |
| B8 | P3 | Address field's leading icon is a magnifier on some loaded pages, a page icon on others | Magnifier only while editing a search; a loaded page shows the page icon | `B10-sticky-header.png` (left vs right) |
| B9 | P3 | Find bar has a field and "6 matches" only | Previous/next (Cmd+G, Cmd+Shift+G), Done, current match highlighted apart from the rest | `B9-find-bar.png` |
| B10 | P3 | Sticky header with CSS `backdrop-filter` shows the content behind it unblurred | Engine support, or the gap recorded | `B10-sticky-header.png` |

### Shell and tooling

| ID | Sev | Finding | Expected | Evidence |
|---|---|---|---|---|
| S1 | P1 | Inactive windows are drawn semi-transparent: stacked windows' text bleeds through each other and the wallpaper | Windows opaque; only materials (sidebar, toolbar, menus) translucent with blur | `S1-translucent-inactive-windows.png` |
| S2 | P3 | `midget.py state` prints `app_controls=...:15:0window=Settings,...` (no newline before the first `window=`) | One key per line | |
| T1 | P3 | `midget.py key` has no punctuation names, so Cmd+[ and Cmd+] cannot be sent | `[ ] / \ , . ; ' - = `` ` keys | |
| T2 | info | `ssh midget` (192.168.20.140, VLAN 20) gave "No route to host"; `midget.py` fell back to another address | Check VLAN 20 | |

## Fix wave

Dispatched 2026-10-19 as a workflow of Sonnet agents, one branch each (`fix/e2e-*`), code and commit only; the lead
reviews, merges into `integration`, builds, deploys to Midget and re-runs this test.
