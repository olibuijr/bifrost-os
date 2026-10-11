# Changelog

## 1.0.3 — 2026-10-11

- The toolbar search field is always open, as in the stock Console (`widgets::search_field`, a labelled text field), so the
  kit publishes it on the first screen for accessibility and `bifrost pc midget tap-window` (was a closed search button
  until focused or filled). Clicking it still focuses it.

## 1.0.2 — 2026-10-11

- Journal follow runs on the utility dispatch queue (`bifrost_app::dispatch`) instead of a raw thread. New lines wake the
  window through the main queue, so the window no longer polls every 200 ms while streaming.
- The app menu is titled Log (Start Streaming, Info and Debug, Find Message, Clear Display); the shell bar already owns
  View, Edit and the rest. Find moves into it with cmd+f.
- The detail pane opens when the window has room for its minimum width and a 380 pt table (was a fixed 640 pt test).
- Timestamps use named microsecond constants.

## 1.0.1 — 2026-10-10

- An entry that claims a subsystem its app does not own loses the subsystem and category and is marked
  `[claimed subsystem …]` (system 0.5.7 registry owners).
## 1.0.0 — 2026-10-10

- New system app, preinstalled and not removable (`kind = "system-app"`, entitlement `log-store`), like Console on the Mac.
- Live (This Mac): the system journal, streamed with Start and Pause, or read as the newest 500 entries. Info and Debug
  messages are hidden until their toggles are on. The table keeps the newest 50,000 entries.
- Search tokens `process:`, `subsystem:`, `category:` and `type:` plus free text; quote a value that has spaces.
- Reports: Crash Reports from `coredumpctl` with a detail pane from `coredumpctl info`, and the files in `/var/log`.
- Golden Gate window: source list, unified toolbar, Light and Dark, semantic colours only.
