# Wire: Browser app (`springboard/apps/browser`, 0.10.0)

Three parity models are now used by the Browser app: the Netscape bookmark export parser (`bookmark_import`), the
per-app web visit model (`web_history`, used as an adapter for ranged history clearing), and the content-rule model
(`content_rules`, not wired, see gaps). The app's compliance findings went from 29 to 0
(`python3 tools/check-app-compliance.py --app browser`).

## What is wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt model | Where in the app |
|---|---|---|---|
| safari-services | Importing data exported from Safari | `springboard/ui/src/bookmark_import.rs` | `library::import_html`, `library::newest_export`, `Store::import_bookmarks` |
| screen-time | Web-history deletion (STWebHistory) | `springboard/ui/src/web_history.rs` | `Store::clear_history_during` (adapter over the Browser's own history) |

Also changed in this round (no model): all 29 app compliance findings (named tokens for colours, radii and spacing
via `src/tokens.rs` and `prefs_layout::m`; `src/inbox.rs` replaces channels and raw threads with the dispatch queues).

## How a user reaches it

- **Import Bookmarks:** Settings (Cmd+,) > first group, fourth row, "Import Bookmarks...". The app reads the newest
  file in `~/Downloads` whose name contains "bookmark" and ends in `.html` or `.htm`, adds the links that are not
  bookmarked yet, and shows the count in the sheet footer ("Imported N bookmarks.").
  - Safari exports a ZIP. Unzip it and copy its bookmarks HTML file into `~/Downloads`. Apple's page says the file is
    the Netscape Bookmarks format, which is what the parser reads.
- **Scripts and MCP:** `browser.bookmarks {op: "import", path: "/path/to/Bookmarks.html"}` returns the bookmarks and
  `imported` (the count added). `browser.history {clear_since: <epoch seconds>}` deletes history last visited at or
  after that time and returns `cleared`.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`). Do not
   build here; this round was written without a build.
2. Put a Netscape bookmark file on Midget: `~/Downloads/Bookmarks.html` with a few `<A HREF="...">` links.
3. Open the Browser, press Cmd+, and tap the "Import Bookmarks..." row (`bifrost pc midget tap-window Browser X Y`,
   window-local pixels from a screenshot). The footer should read "Imported N bookmarks." Cmd+Option+B shows them.
4. Run it again: the footer should read "Imported 0 bookmarks.".
5. `browser.bookmarks {op: "import", path: ...}` over the control socket (`bifrost pc control call`) returns the same count.
6. History range: `browser.history {clear_since: <now minus 1 hour>}` removes only recent entries, returns `cleared`.

Unit tests (run by the lead, not run here): `store::import_tests`, `library::export_tests`, `tokens::tests`,
`inbox::tests`, and the existing `library`, `downloads`, `favicon` tests (the probe and icon results now arrive through
slots, so their tests exercise the same functions).

## REA evidence

- `ev_bcc967b646914f3088626daa110b1866316cbafd034e552ce211b8eb14fc324c`: official page "Importing data exported from
  Safari" (rendered). It says Safari exports bookmarks as an HTML file in the Netscape Bookmarks format, inside a ZIP
  with the passwords, payment cards and per-profile history (JSON) files. It also says the history is JSON with
  visited-site objects.
- `docc.py safariservices/importing-data-exported-from-safari`: same page, text only (no evidence id).

## Remaining gaps

- **Content rules / content blockers (`content_rules`):** not wired. The browser engine (`apps/browser/engine`) has no
  per-request hook, so a rule list cannot block a load. Needs an engine change first.
- **History range in Settings:** only the control method and MCP action take a range. The Settings "Clear History..."
  alert still clears everything.
- **Import scope:** bookmark links and titles only. Folder names, the Reading List folder, saved passwords (CSV),
  payment cards, extensions and the JSON history file are not imported.
- **web_history is an adapter, not shared history:** the Browser keeps its own history file. Each history entry stands
  for its last visit, so a range clear removes an entry whole when its last visit is in the range.
- **Not built or run:** this round was written without cargo, rustc or tests, per the wiring rules. The lead builds once.
