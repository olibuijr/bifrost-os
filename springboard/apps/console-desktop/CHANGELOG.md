# Changelog

## 1.2.0 — 2026-10-12

- Security Events: a new row under Reports lists the endpoint security events BifrOSt can subscribe to: socket bind and
  connect, socket suspend and resume, pseudoterminal grant and close, set time, and kernel extension load and unload.
  Each row says its category, whether it waits for a verdict before the action proceeds, and whether the subscription
  wants it. Search filters the list: `category:Socket` keeps one category, and `not` finds the events the subscription
  lacks. The detail pane explains the event.
- No event feed yet: nothing reports live security events, so the list is the subscription's contents, not a log of
  what happened. The footer says so.
- Code requirements: the detail pane of a live message or a signpost shows its process's executable, its SHA-256 (the
  first 16 digits), whether it has a valid code signature, and whether it meets the unsigned and platform
  requirements. BifrOSt does not sign code yet, so programs are unsigned and fail the platform requirement. A program
  over 64 MB is not hashed.

## 1.1.0 — 2026-10-12

- Log positions: every message has a position in the log's order (shown as Position in the detail pane). Log, Show
  Messages After Selected lists only the messages newer than the selected one, and Show All Messages returns to the
  full list. The walk keeps up with new messages while the journal streams.
- Subsystem filter: Log, Show Only This Subsystem adds a `subsystem:` search token for the selected message's subsystem.
- Signposts: a new Signposts row under Reports lists the signpost intervals and events in the journal. Each interval's
  duration is in the detail pane, and the footer totals every name (count, total, longest). The Console times its own
  view loads as a signpost (`view.load`) in its log subsystem.
- Activity filters are not available yet: journal lines carry no activity identifier.
- Power: the footer shows the machine's thermal state, the battery (charge, charging or discharging, and the time left) and
  Low Power Mode, from the system power service. It updates when the machine reports a change.
- Copy Message: Log menu copies the selected line (time, process, level, subsystem, category and message) to the
  clipboard, through the system pasteboard service. An older system copies it to the plain clipboard instead.
- Energy impact per process is not shown yet: the system does not measure energy per process.

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
