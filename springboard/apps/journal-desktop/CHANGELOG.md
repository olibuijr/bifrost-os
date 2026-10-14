# Changelog

## 0.1.4 — 2026-10-14

- Drop the unused documents permission; Journal uses only app storage.

## 0.1.3 — 2026-10-14

- Draw the entry timeline with the shared Golden Gate source-list rows.

## 0.1.2

- Added the SDK and design compliance test harness.

## 0.1.1

- Exported the app icon at the required 180 x 180 pixel size.

## 0.1.0

- First version: dated journal entries with text, a timeline of entries (newest first), search over the text and
  dates, bookmarks with a bookmarks-only filter, and photos attached by copying an image file into the app's data
  directory.
- Entries are saved to `journal.json` in the app data directory on every change, through a temporary file and a rename.
