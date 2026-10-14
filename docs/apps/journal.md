# Journal

Journal (`springboard/apps/journal-desktop`, crate `bifrost-journal`, version 0.1.0, app id
`is.olibuijr.journal.desktop`) is a private journal. Each entry has a date, a text and optional photos.

## What it does

- **Entries by date.** New Entry makes an entry for today (UTC) and selects it. An entry can be edited at any time.
- **Timeline.** The sidebar lists the entries newest day first and, within a day, newest entry first. Each row shows
  the date, the first line of the text, whether the entry is bookmarked and how many photos it has. Drag the list to
  scroll; tap a row to open it.
- **Search.** The field at the top of the sidebar matches every word of the query against the entry text and the
  date (`2026-10-13`), ignoring case. Escape clears it.
- **Bookmarks.** The Bookmark button marks the open entry. The "Bookmarks Only" button limits the timeline to
  bookmarked entries.
- **Photos.** Type the path of an image file (jpg, jpeg, png, gif, webp or heic) in the photo field and choose
  Attach Photo (or press Return in the field). The file is copied into the app's data directory; the original is not
  touched. Photos are listed in the photo bar and counted in the timeline.
- **Delete.** Delete removes the open entry and the copies of its photos.

## Where the data lives

The app keeps everything in its data directory (`BIFROST_DATA`):

- `journal.json`: the entries (id, day, created and modified Unix seconds, text, photo file names, bookmark flag)
  and the next free id. The search, the filter and the selection are not saved.
- `photos/`: the copied image files, named `<entry id>-<n>.<ext>`.

Every change is written at once: the file is written to `journal.json.tmp` and renamed over `journal.json`. A file
that cannot be read or decoded is moved aside as `journal.json.bad` and the journal starts empty, so existing entries
are never silently overwritten.

## Code layout

- `src/model.rs`: the journal as data (entries, day arithmetic, timeline order, search, bookmarks, photo names) and
  the JSON format. No drawing and no file access.
- `src/store.rs`: load and save of `journal.json`, copying and deleting photo files.
- `src/layout.rs`: where things sit, in points, and the touch hit tests.
- `src/main.rs`: the window: drawing through the SDK kit, touch and key handling.

## Limits of this version

- Photos are stored and listed by file name; they are not drawn as thumbnails yet.
- The photo is chosen by typing its path, not from a file picker.
- Days are UTC. An entry made late in the evening in a time zone east of UTC can land on the next day; local
  time needs the shell's clock service.
- Text is wrapped by word for display; runs of spaces are shown as one space.
- Edit > Copy and Edit > Paste are not wired yet.
- The icon is a copy of the Sticky Notes icon and should be replaced with the Journal icon.
- Every keystroke rewrites `journal.json`. Fine for a personal journal of this size; a large journal would want a
  debounced save.

## Tests

Unit tests cover the day arithmetic, timeline order, search, bookmarks, photo names, the JSON round trip and skipping
of malformed entries (`model.rs`), the file store (`store.rs`) and the hit tests (`layout.rs`).
