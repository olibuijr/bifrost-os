# Browser daily polish (slug browser-daily)

Scope: tabs and tab overview, bookmarks bar and manager, history with search, downloads list, Reader mode, password
autofill from the shared credentials store, private window, find in page. Code: `springboard/apps/browser`.

Evidence: no reference extraction was used this round (`~/Projects/macos-ref/extract/ref27/` holds no Safari or
browser bundle yet, and no REA evidence ids were taken). Behaviour comes from the existing browser code and its README.

## Status per item

| Item | State | BifrOSt path | Notes |
|---|---|---|---|
| Tabs | Existing | `apps/browser/src/main.rs` (tab strip, drag reorder) | Unchanged. |
| Tab overview | New (0.14.0) | `apps/browser/src/overview.rs`, `main.rs` (`toggle_overview`, touch and Escape branches, `draw`) | View > Show All Tabs, Cmd+Shift+backslash. Text cards only: the engine keeps no snapshot, so no thumbnails. Needs Midget check of the grid and the touch hit. |
| Bookmarks bar | Existing | `apps/browser/src/library.rs` (favorites bar, overflow menu) | Unchanged. |
| Bookmarks manager | Partial | `apps/browser/src/library.rs` (Bookmarks/History panel, Delete, search) | No folder management. Open gap. |
| History with search | Existing | `apps/browser/src/library.rs` (History mode, search field, grouped by day) | Unchanged. |
| Downloads list | Existing | `apps/browser/src/downloads.rs` | Unchanged. |
| Reader mode | Existing | `apps/browser/src/reader.rs` | Unchanged. |
| Password autofill | New (0.14.0) | `apps/browser/src/autofill.rs`, `main.rs` (`autofill_page`, `service_reply`, `LoadFinished` eval) | File > AutoFill Password. Uses `bifrost_app::passwords` (list, fill, decode_list, decode_password); manifest adds `keychain` and `keychain_groups = ["is.bifrost.passwords"]`. Manual trigger only: no automatic fill and no save prompt. Needs Midget check of the fill script on a real login form. |
| Private window | Existing | `apps/browser/src/main.rs` (`--private`, Cmd+Shift+N) | Unchanged. |
| Find in page | Existing | `apps/browser/src/findbar.rs` | Unchanged. |

## Unit tests added

- `autofill.rs`: host matching, newest login wins and notes are skipped, JS string escaping, fill script contents, form flag replies.
- `overview.rs`: grid column count, card wrapping and hit testing, address shortening, card labels.

## Still a model or needs Midget

- Tab overview layout and hit testing on a real window size (drawn from the same kit calls as the downloads popover).
- Autofill on a live login form: the field choice (last text-like field before the password field) and the event dispatch.
- Bookmark folders, password save prompt, page thumbnails in the overview.
