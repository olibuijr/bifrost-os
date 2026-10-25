# Changelog

## 0.1.9

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.8

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.7

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.6

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.5

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

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
