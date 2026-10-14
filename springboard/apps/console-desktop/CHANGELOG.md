# Changelog

## 1.7.1 — 2026-10-14

- Find Next, Find Previous and Use Selection for Find are now in a dedicated Find menu, avoiding a duplicate shell Edit menu.

## 1.7.0 — 2026-10-14

- View menu: Show Only This Activity narrows the table to the systemd unit invocation of the selected message, and
  Show All Activities undoes it. Messages without an invocation leave the item disabled. The grouped Activities
  outline is not built yet.
- View menu: Hide Toolbar removes the toolbar's items and search field, and Show Toolbar brings them back. Customize
  Toolbar is not built yet.
- Files the shell hands over, such as a log dropped on Console or opened with it, open under Opened Files. Plain text
  (which covers .log) and gzip archives are declared as types Console can open.
- Not in this version: bzip2 and xz files, opening from the command line, Open Recent across sessions, and Reveal or
  Trash for the files in /var/log and for crash reports.

## 1.6.0 — 2026-10-13

- View menu: Reload (Cmd+R) reads the current view again. Show Sidebar and Show Details hide and show those panes.
  Visible Columns chooses which columns the table shows; PID is new and starts off. Expand Message shows the selected
  message whole in the table, Collapse All Messages undoes every expansion.
- Edit menu: Find Next (Cmd+G) and Find Previous (Shift+Cmd+G) move the selection to the next or previous message that
  matches, wrapping at the ends, without filtering the table. The match is the find text, or the search field when
  there is none. Use Selection for Find (Cmd+E) takes the selected message's first line as the find text.
- File menu: Open reads a log file, a gzip-compressed log or a saved archive and lists it under Opened Files in the
  sidebar; Open Recent switches between the ones opened in this session. Capture Archive saves this boot's journal in
  the journal export format to a place chosen in the save panel and opens it. Reveal in Files and Move to Trash work
  on an opened or captured file.
- Save Messages now asks where to save in the save panel. A shell without the panel gets the old file in
  Documents/Console.
- Reports has a Kernel Reports row: the crash records the kernel keeps in the persistent store across a reboot, and the
  previous boot's kernel warnings and errors. Selecting a record shows its text.
- Not in this version: bzip2 and xz files, opening from the command line or by double-click, a list of other devices,
  toolbar customization, and Reveal or Trash for the files in /var/log and for crash reports, which the app holds no
  grant for.

## 1.5.0 — 2026-10-13

- Save Messages (Log menu): writes the selected message, or every message the window shows when none is selected, to a new
  `Console-<time>.log` in Documents/Console. A save never overwrites an earlier file. The result shows in the status line.

## 1.4.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.4.1 — 2026-10-12

- Removed `Query::is_empty` and `Buffer::is_empty` from the release build. Only the tests asked for them, so they are
  compiled under `cfg(test)` now. Nothing a user sees changes.

## 1.4.0 — 2026-10-12

- Reports has two new rows. Hang Diagnostics lists each time the shell's UI loop stopped answering for 20 seconds or
  more, with how long it stalled. Its detail says that no call stack is kept for a hang. Daily Summary counts the crash
  reports and hangs for each day (UTC), and its rows say how many of each happened and the stall time added up.
- Crash Reports is unchanged: its detail still shows the call stacks that systemd-coredump kept.
- Not in this version: metric reports from the shell's per-app counters, and opening or saving log archives. The
  shell does not send its counters to the Console, and BifrOSt has no log archive format yet.

## 1.3.0 — 2026-10-12

- Saved searches: Log, Save Search keeps the text in the search field as a row in a Saved Searches section of the
  sidebar. Choosing the row shows the live journal with that search in the field. Log, Delete Saved Search removes the
  row for the text in the search field. Saved searches last until the Console quits; they are not written to disk yet.
- Search tokens: `level:error` works like `type:error`. A new `message:` token keeps only the messages whose text
  contains its value (process, subsystem and category are not checked for it).

## 1.2.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

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
