# Wire 4: Calendar data detection (bifrost-calendar 1.10.0)

Branch `wire4/calendar-detect`. Scope: `springboard/apps/calendar-desktop/` (version and changelog), plus this note.
No build or test was run in this round (wave rule); the lead builds once. The matrix data (`docs/parity-matrix/`) is
not edited; the lead updates the rows after the build.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status |
|---|---|---|---|
| `datadetection` | Calendar events and dates | `springboard/apps/calendar-desktop/src/calendar/quick.rs` (`parse`), used by `Calendar::new_event_from_text` in `src/calendar.rs`; File menu in `src/main.rs` | wired |
| `datadetection` | Phone numbers | `springboard/apps/calendar-desktop/src/calendar/notes.rs` (`phone`), editor row `Target::CopyPhone` in `src/calendar.rs`, pasteboard hand-off in `src/main.rs` (`share_copy`) | wired |
| `appkit-text` | Text Display: data detectors | `springboard/apps/calendar-desktop/src/calendar/notes.rs` (`address`, `phone`) over `springboard/ui/src/detect.rs` (`find`, `Kind::Address`, `Kind::Phone`) | wired (addresses from wire 3; phone numbers added here) |

Both `datadetection` rows stay `partial` in the matrix until verified on Midget: the sentence parser is English only and
reads one line, and only the Calendar app uses it.

## How a user reaches it

- New Event from Text: copy a sentence (for example from a message: "Lunch with Ana next Tuesday at 12:30 at
  Laugavegur 5"), then choose File > New Event from Text. The editor opens with:
  - Title: "Lunch with Ana" (the words before the first scheduling phrase).
  - Date: the next Tuesday after today (a bare weekday means the next one, today included).
  - Start: 12:30. Duration 1 hour.
  - Location: "Laugavegur 5" (the shared address detector).
  Save writes the event; Cancel discards it. With nothing copied, the command opens a plain New Event.
  The command is disabled while an event is open.
- Phone numbers in notes: write "Call +354 555 1234 before lunch" in the Notes field. The row "Copy +354 555 1234"
  appears under Notes (below "Use ... as Location" when an address is also there). Tapping it copies the number to the
  pasteboard and shows "Copied +354 555 1234".

Sentence rules (`src/calendar/quick.rs`):

| Phrase | Result |
|---|---|
| `today`, `tonight`, `tomorrow` | that day; `tonight` with no time starts at 19:00 |
| `next Tuesday` (said on a Tuesday is a week on), `this Friday`, `on Monday`, `Friday` | the weekday after today (`Friday` alone counts from today, unless it is the first word) |
| `2026-10-20`, `20.10.2026` | that date (real calendar dates only) |
| `at 12:30`, `at 9:00`, `3pm`, `5 pm`, `noon` | the start time |
| `at Laugavegur 5` | the location (first street address in the line) |

Words before the first phrase are the title, so "Dinner at Ana's tomorrow" keeps "Dinner at Ana's" as the title
("at" is a place only before an address or a time).

## How to verify on the test PC (Midget)

Host tests first (run by the lead; not run in this round):

    cargo test --offline -p bifrost-calendar quick
    cargo test --offline -p bifrost-calendar notes
    cargo test --offline -p bifrost-calendar new_event_from_text
    cargo test --offline -p bifrost-calendar phone_number
    cargo test --offline -p bifrost-calendar compliance

Midget:

1. `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.calendar.desktop`.
2. Copy the sentence "Lunch with Ana next Tuesday at 12:30 at Laugavegur 5" on the desktop (any text field, select, Cmd+C).
3. Shell File menu > "New Event from Text". The editor shows Title "Lunch with Ana", Location "Laugavegur 5", the
   start 12:30, and the next Tuesday in the date row. Save; the event is in the month grid.
4. New Event, Notes "Call +354 555 1234 before lunch". The row "Copy +354 555 1234" shows under Notes in the link colour.
   Tap it (`bifrost pc midget tap-window APP X Y`, window-local px); the notice reads "Copied +354 555 1234". Paste
   elsewhere to confirm the number arrived.
5. With nothing copied (empty pasteboard), File > New Event from Text opens a blank New Event at 9:00.
6. Check the screenshot with `bifrost-hig` (rules CNT-01, COL-01, COL-02, GG-05); `bifrost pc midget screen PATH`.

## REA evidence (official documentation, rendered page)

- `ev_42baed5600c7e7b4d279ae14d297898b0d9ae0ce4aa5301d500d123f231b542c`: NSDataDetector
  (foundation/nsdatadetector), rendered this round. The page: the data detector finds dates, addresses, links, phone
  numbers and transit information in natural language text, returns results as checking results, and "discards
  potential matches in case of uncertainty", so it is not for validation.
- `ev_c2460bd48a41beca2c111912585b5fba0d8769055553a0cf35d80a7e38beb2bc`: NSTextCheckingResult
  (foundation/nstextcheckingresult), rendered this round. DocC lists the date result (`dateCheckingResult(range:date:)`,
  with `timeZone` and `duration`) and the phone result (`phoneNumber`), which the calendar's rules follow in shape:
  a date with a start time and a duration, and a phone number as written.
- DocC: `python3 -I tools/parity/docc.py foundation/nsdatadetector` and `... foundation/nstextcheckingresult`.

## Remaining gaps

- The sentence parser is calendar-local (`src/calendar/quick.rs`). `springboard/apps/reminders-desktop/src/quick.rs` has
  a similar English quick-add parser for reminders. Moving both onto one shared model (in `springboard/ui`) is the
  right next step, outside this app's scope.
- Date-only and time-only phrases in notes do not offer an event ("Lunch on the 20th" in notes is not detected). Only
  the New Event from Text command reads a sentence.
- A bare weekday that is the first word is treated as part of the title ("Friday drinks").
- English only: no Icelandic weekday or month names ("þriðjudagur", "á morgun") in the sentence parser.
- Phone numbers: the action is copy only. No dial action exists on the PC (no telephony service in the shell).
- Time zones: all times are local; a sentence cannot name another zone.
- Availability (busy, free, tentative) and Up Next relevance remain open as listed in `docs/parity/wire3-calendar.md`.

## Matrix rows

| Slug | Feature | BifrOSt path |
|---|---|---|
| `datadetection` | Calendar events and dates | `springboard/apps/calendar-desktop/src/calendar/quick.rs` |
| `datadetection` | Phone numbers | `springboard/apps/calendar-desktop/src/calendar/notes.rs` |
| `appkit-text` | Text Display: data detectors | `springboard/apps/calendar-desktop/src/calendar/notes.rs` |
