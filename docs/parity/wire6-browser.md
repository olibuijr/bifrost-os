# Wire 6: Browser app (`springboard/apps/browser`, 0.13.0)

Sixth wiring round for the Browser, focused on web content: downloads, content blockers, user scripts, find in page,
page zoom and print. Most of the focus was already built in earlier rounds: the Downloads popover with progress,
reveal, open and clear (`downloads.rs`), the find bar (`findbar.rs`, Cmd+F, Cmd+G), zoom (`session.rs` `zoom_by`,
Cmd+Plus, Cmd+Minus, Cmd+0) and the Reader (`reader.rs`). This round puts those on the menu bar, where a user can find
them, and records what could not be wired. No cargo, rustc or tests were run (the lead builds once).

## What is now wired

| Piece | Model | Where in the app |
|---|---|---|
| View > Zoom In, Zoom Out, Actual Size | none (the Browser's `zoom_by`, which remembers the zoom per site) | `main.rs`: `menus()` (View menu, keys `cmd+=`, `cmd+-`, `cmd+0`, greyed out without a page), `menu_action` arms `MENU_ZOOM_IN`, `MENU_ZOOM_OUT`, `MENU_ACTUAL_SIZE` |
| View > Find in Page, Find Next, Find Previous | none (the Browser's find bar and `WebProcessPool::find`) | `main.rs`: `menus()` (keys `cmd+f`, `cmd+g`, `cmd+shift+g`; Next and Previous greyed out while the bar is closed), `menu_action` arms `MENU_FIND`, `MENU_FIND_NEXT`, `MENU_FIND_PREV` |
| File > Show Downloads | none (the Browser's `Downloads` list) | `main.rs`: `menus()` (File menu, after a separator; greyed out with no downloads), `menu_action` arm `MENU_DOWNLOADS` opens the existing popover |
| File > Clear Finished Downloads | none | `main.rs`: `menus()`, `menu_action` arm `MENU_CLEAR_DOWNLOADS` calls `Downloads::clear_finished` (removes finished and failed entries, never the files) |

### Why the Find items sit in View, not Edit

The compliance gate (`tools/check-app-compliance.py`, rule CHR-01) rejects an app menu titled Edit: the shell's Edit menu
already holds the standard items, so an app's own Edit would be a second one. Only File, View, Window and Help merge.
Find is therefore in View, after the zoom items. This is a placement choice; a shell-level Find in Edit would be the
reference layout and is listed under gaps.

### Compliance

`python3 tools/check-app-compliance.py --app browser` reports 0 findings. Two findings were fixed on the way: the Edit
menu (see above), and the History menu's id base constant was renamed `NAV_MENU_BASE` (it was `HISTORY_BASE`; the
checker read the name as a fixed resolution because it starts with `h`). The app's `CONTENT_BG` fill is unchanged.

## How a user reaches it

- **File > Show Downloads** (or the Downloads toolbar button, which appears after the first download). The popover lists
  each download with its progress; Reveal, Open, Cancel and Clear are in the popover rows.
- **File > Clear Finished Downloads** removes the completed, failed and cancelled entries from the list.
- **View > Find in Page...** (Cmd+F) opens the find bar under the tab strip. Type to search. Enter or Cmd+G goes to the
  next match, Cmd+Shift+G to the previous one (View > Find Next and Find Previous). Esc closes the bar.
- **View > Zoom In** (Cmd+=), **Zoom Out** (Cmd+-) and **Actual Size** (Cmd+0). The zoom is kept for the site.

## How to verify on the test PC (Midget)

1. Build and publish (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open a page with text and links. Open View: Zoom In, Zoom Out and Actual Size are enabled. Choose Zoom In twice: the
   page grows; Actual Size returns it to 100 percent. Reopen the page: the zoom is still there for that site.
3. Choose View > Find in Page: the find bar opens with the field focused. Type a word on the page: the match count
   appears. Choose View > Find Next, then Find Previous: the page scrolls between matches. Esc closes the bar, and
   View > Find Next is greyed out again.
4. Open File: "Show Downloads" is greyed out until a file has been downloaded. After a download (a link to a `.zip`
   or `.pdf`), it is enabled and opens the popover. "Clear Finished Downloads" is enabled once a download is done;
   choosing it empties the list and the files stay in `~/Downloads`.
5. Screenshot with `bifrost pc midget screen PATH`, check the menus with the bifrost-hig checklist (standard items,
   separators, shortcuts shown), and check Light and Dark (`bifrost pc midget appearance light|dark`).

Unit tests (not run; the lead runs them): `main::host_tests::menu_ids_are_distinct_and_below_the_history_range`
(ten menu ids, each distinct and between 1 and `NAV_MENU_BASE`). The existing `tabdiscard` tests cover the History
menu ids after the rename. The compliance snapshot (`tests/snapshots/browser-*.png`) may change if it captures the menu
bar, so the lead should check it and bless it if the change is intended.

## REA evidence

- `ev_9511913bd46bc0b1ee6b299abcbdc6e96ddb920ea9308962cc24aab340a84389`: official page "WKFindConfiguration" (rendered
  with `tools/parity/rea-page.sh`). It says the configuration sets how to search the web view's contents (backwards,
  case sensitivity, wrapping), and `docc.py webkit/wkfindconfiguration` lists `backwards`, `caseSensitive` and `wraps`.
  The find bar uses direction only; case and wrap options are not exposed by the engine (see gaps).
- `ev_c27960f40091ad880340d8f39fed3add7adfac0cd8ddc703446c7effa507b85c`: official page "pageZoom" (rendered). The scale
  factor by which the web view scales content relative to its bounds. Zoom uses this factor.
- `ev_4d4c81a5c1f3601199ce2d026180647a822cd9c8712f55cf22c44b15de1071d0`: official page "WKContentRuleList" (rendered).
  It is the compiled list of content rules the web view applies to requests. Used for the gap below.
- `docc.py webkit/wkfindresult` (no evidence id): "matchFound" says whether the web view found a match. The find bar
  shows "Not found" or the match count from the engine's result.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status after this round |
|---|---|---|---|
| webkit | Page-level search | `springboard/apps/browser/src/main.rs` (`menus`, `menu_action`: View > Find in Page, Find Next, Find Previous); `springboard/apps/browser/src/findbar.rs` | still `partial` (no case or whole-word option, no match highlight) |
| webkit | Web views and configuration | `springboard/apps/browser/src/main.rs` (`menus`, `menu_action`: View > Zoom In, Zoom Out, Actual Size); `springboard/apps/browser/src/session.rs` (`zoom_by`) | still `partial` (zoom is wired; the other configuration options are not) |
| webkit | Downloads | `springboard/apps/browser/src/main.rs` (`menus`, `menu_action`: File > Show Downloads, Clear Finished Downloads); `springboard/apps/browser/src/downloads.rs` | still `partial` (the Browser's own downloads; no `WKDownload` delegate in the SDK) |
| safari-services | Content blockers | none wired (`springboard/ui/src/content_rules.rs` is a model with block and allow rules only) | still `partial` |

## Remaining gaps

- **Content blocker list in Settings (not built):** the rule model (`springboard/ui/src/content_rules.rs`) has block and
  allow rules but no cosmetic (hide element) rules, and the engine has no per-request hook, so the Browser cannot stop
  a request before it is sent (the engine's `stop` is a no-op; see the `downloads.rs` note). A block list in Settings
  would not block anything, which would be a dead control, so none was built. A top-level navigation check would stop
  only the main document, not the ads and trackers the list is for. Needs an engine hook first (the `WKContentRuleList`
  page above is the reference for the compile-and-apply step).
- **Print (not built):** the engine has no print path for a page, and the shell's Print sheet (`print_sheet.rs`) prints
  the desktop, not a web page. Needs an engine command that renders the page to PDF.
- **User scripts (not built):** the matrix row "Page content, scripts and messaging" is not in this round's focus, and
  the engine injects only the Browser's own scripts. No user script list was added.
- **Find options:** case sensitivity and whole-word matching are not offered; the engine call takes direction only.
- **Find placement:** Find is in View because of the Edit menu rule. A shell-level Find in Edit would be the reference
  layout.
- **Not built or run:** no cargo, rustc or tests were run in this round, per the wiring rules. The matrix data under
  `docs/parity-matrix/` is not edited; the lead updates it after the build and the Midget check.
  `springboard/Cargo.lock` still names the app as 0.12.0 until the lead's build updates it.
