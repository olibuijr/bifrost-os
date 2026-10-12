# Wire 3: Calendar app (bifrost-calendar 1.9.0)

Branch `wire3/calendar`. Scope: `springboard/apps/calendar-desktop/` only, plus this note. The matrix data
(`docs/parity-matrix/`) is not edited; the lead updates the rows after the build. No build or test was run in this
round (wave rule); the lead builds once.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status |
|---|---|---|---|
| `appkit-text` | Text Display: data detectors | `springboard/ui/src/detect.rs` (`Kind::Address`), used by `springboard/apps/calendar-desktop/src/calendar/notes.rs` and the editor in `src/calendar.rs` | wired |

Behaviour: when an event's notes (live text, before Save) contain a street address, the editor shows a row
"Use <address> as Location" directly under Notes. Tapping it sets the Location field to the first address found.
The detector is the shared `bifrost_app::controls::detect` (the same rules the Notes app uses), so there is no
second address rule in the calendar.

Also changed in this round:

- Background: the main content area of the month, week, day and year views (and the whole window fill behind them)
  uses `CONTENT_BG` (white in Light, dark grey in Dark) instead of `DETAIL_BG` (pure black in Dark). Files:
  `src/calendar/chrome.rs` (window fill), `src/calendar/timeview.rs` (grid and header fills).
- Version 1.9.0 in `Cargo.toml` and `manifest.toml`; `CHANGELOG.md` entry at the top.

## What was already in the app (verified in code, not changed)

- Repeats: `src/calendar/recur.rs` (RRULE subset, `EXDATE` exceptions). The editor's Repeat row cycles
  `store::REPEATS`. Matrix `eventkit` "Recurrence rules" stays `partial`.
- Alarms: `src/calendar/alerts.rs` (`ALERT_MINUTES`, due alerts through the notification schedule). Matrix `eventkit`
  "Alarms" stays `partial`.
- Up Next: `src/calendar/connect.rs` (`up_next_entries`), start-ordered today's timed events.

## How a user reaches it

Calendar (Dock or Launchpad) > New Event (Cmd+N) > type a title > in Notes write an address such as
"Table for two at Laugavegur 12" > the row "Use Laugavegur 12 as Location" appears under Notes > tap it > Location
is filled > Save.

## How to verify on the test PC (Midget)

Host tests first (run by the lead; not run in this round):

    cargo test --offline -p bifrost-calendar an_address_in_the_notes
    cargo test --offline -p bifrost-calendar notes
    cargo test --offline -p bifrost-calendar compliance

Midget:

1. `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.calendar.desktop`.
2. New Event, title "Dinner", Notes "Table for two at Laugavegur 12". The row "Use Laugavegur 12 as Location" shows
   under Notes and is drawn in the link colour. Tap it (`bifrost pc midget tap-window APP X Y`, window-local px).
   Location now reads "Laugavegur 12". Save.
3. Without an address in Notes, the row is absent.
4. Background: `bifrost pc midget appearance light`, screenshot with `bifrost pc midget screen /tmp/calendar-wire3.png`;
   the content area is white. Then `bifrost pc midget appearance dark`; the content area is dark grey, not black.
5. Check the screenshot with `bifrost-hig` (rules CNT-01, COL-01, COL-02, GG-05).

## REA evidence (official documentation, rendered page)

- `ev_3c7bd6c3a1c89908d050a5b61f31e7422e31a2f11488c470fd334607bc4b4b71`: NSDataDetector
  (foundation/nsdatadetector), rendered this round. The page states the detector is a specialized regular expression
  for natural-language data patterns, and that it discards uncertain matches.
- DocC read with `python3 -I tools/parity/docc.py foundation/nsdatadetector`.
- Earlier wave evidence for the calendar rows is in `docs/parity/wire-app-calendar.md` (`ev_fc582b...` EKRecurrenceRule,
  `ev_01cac1...` UNCalendarNotificationTrigger).

## Remaining gaps

- Up Next relevance time clues: `springboard/ui/src/relevance.rs` (`TimeClue`, `rank`) is a pure model, but the shell's
  timeline entry has no relevance field (no `relevance` in `springboard/src` or `springboard/core`). The app cannot
  rank Up Next by clue until the shell timeline wire carries a relevance hint. Shell change needed: outside this
  app, not made here.
- Availability (busy, free, tentative, unavailable): no availability model exists in `springboard/ui` or
  `springboard/sdk`, and the store has no field for it. Matrix `eventkit` "Events" lists "availability" as missing.
  Not wired.
- Date detectors in notes: only addresses are offered. Dates in notes are detected by the shared detector, but the
  calendar has no "create event from this date" action yet.
- Recurrence and alarm editing: unchanged from wire-app-calendar (no "this event only" or "all future events" span,
  no absolute or proximity alarms in the editor).
- Inspector: the app has an event editor and no separate inspector pane; the Location row is the only place the
  detected address lands.
