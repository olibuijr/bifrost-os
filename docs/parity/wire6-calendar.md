# Wire 6: Calendar event store parity (bifrost-calendar 1.11.0)

Branch `wire6/calendar`. Scope: `springboard/apps/calendar-desktop/` (version, changelog, code), plus this note.
No build or test was run in this round (wave rule); the lead builds once. The matrix data (`docs/parity-matrix/`)
is not edited; the lead updates the rows after the build. `springboard/Cargo.lock` still lists `bifrost-calendar`
at 1.10.0; the next build updates that entry (the lock file is outside this app's folder, so it was not edited).

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path | Status |
|---|---|---|---|
| `eventkit` | Participants | `springboard/ui/src/calendar_participant.rs` (`Participant`, `Role`, `Kind`, `Status`), read by `springboard/apps/calendar-desktop/src/calendar/invitees.rs`, shown in the editor (`src/calendar.rs`, row `Target::Invitees`) | wired (read-only) |

What changed in behaviour:

- An event that came from an imported calendar file (`calendar.ics` beside the events store) now carries its
  attendees. The importer reads each `ATTENDEE` line (CN, ROLE, CUTYPE, PARTSTAT) into a `Participant`.
- Opening an existing event whose title, start time (or all-day flag) and date match an imported event with
  attendees shows a read-only row under Notes: "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)".
  Three names are listed, then "and n more". Responses: Accepted, Declined, Maybe, Awaiting reply, Delegated,
  Completed, In progress, No reply.
- Attendees are written back as `ATTENDEE` lines when the sidecar `calendar.ics` is saved again, so an import
  does not lose them. Export of a whole calendar (`exchange::build_export`) keeps them too, as they are rich events.
- The row is read-only: tapping it changes nothing and the draft stays open. The accessibility tree publishes it
  as static text labelled "Invitees" (id 211).

Matching rule (`invitees::for_event`): the same rule as the export's de-duplication: same title, same all-day flag,
same start minute for timed events, and an occurrence on the event's date. The first match with attendees wins.

## How a user reaches it

Click an event in the month grid to open its editor. When the event is one of the imported events kept in the
sidecar `calendar.ics` (beside the events store file, `exchange::rich_path`) and that event has attendees, the
Invitees row is under Notes (below "Copy <phone>" when that is also offered).

Import gap: `Calendar::import_ics` (`src/calendar/exchange.rs`) parses the file and writes the sidecar, but no menu
item or shell path calls it in this build (`grep import_ics` finds no caller in the app). So on Midget the sidecar
must already hold the attendees, for example copied there by a test. Wiring an Import menu item is the next step.

Only events that came from a file with `ATTENDEE` lines show the row. A plain event created in the app has no
invitees and no row.

## How to verify on the test PC (Midget)

Host tests first (run by the lead; not run in this round):

    cargo test --offline -p bifrost-calendar invitees
    cargo test --offline -p bifrost-calendar imported_events_invitees
    cargo test --offline -p bifrost-calendar store_changes
    cargo test --offline -p bifrost-calendar compliance
    cargo test --offline -p bifrost-ui calendar_participant

Expected host results (worked out by hand from the code):

- `summary` of two people (Ana Lind, Accepted; bo@example.is, no name, no reply) is
  `Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)`.
- `summary` of five tentative people lists three, then `and 2 more`.
- An `ATTENDEE;CN="Ana, Lind";ROLE=OPT-PARTICIPANT;CUTYPE=INDIVIDUAL;PARTSTAT=TENTATIVE:mailto:ana@example.is`
  line parses to Optional, Person, Tentative, name "Ana, Lind" and writes back as the same line.

Midget:

1. Write an `.ics` file with a timed event "Dinner" at 09:00 on the event's date and two attendees
   (`ATTENDEE;CN=Ana Lind;PARTSTAT=ACCEPTED:mailto:ana@example.is` and `ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:bo@example.is`).
   Copy it to the sidecar path beside the events store on Midget (`calendar.ics`, see `exchange::rich_path`), and
   save the event "Dinner" at 09:00 in the app so the title and time match.
2. `bifrost pc midget ota` until `phase=accepted`, then `bifrost pc midget open-app is.olibuijr.calendar.desktop`.
3. Open the "Dinner" event (click it in its cell). The row
   "Invitees: Ana Lind (Accepted), bo@example.is (Awaiting reply)" shows under Notes, in secondary text.
4. Tap the row (`bifrost pc midget tap-window APP X Y`, window-local px): nothing changes and the editor stays open.
5. Open a plain event created in the app (with a different title): no Invitees row.
6. Screenshot with `bifrost pc midget screen PATH`, check with `bifrost-hig` (CNT-01, COL-01, COL-02, GG-05).
   Check the content area stays `CONTENT_BG` in Light and Dark (it already does, from wire 3).

## REA evidence (official documentation, rendered page)

- `ev_f0f3a3ccf9dc0fb036c888029408c4844b3494cdd32483d85425729397fd229c`: EKParticipant (eventkit/ekparticipant),
  rendered this round. The page says the class "represents person, group, or room invited to a calendar event",
  and that EventKit "cannot add participants to an event nor change participant information". The read-only
  model (no setters) and the read-only row follow that statement.
- DocC: `python3 -I tools/parity/docc.py eventkit/ekparticipant` lists the properties used here: `name`,
  `url`, `participantRole`, `participantType`, `participantStatus`, `isCurrentUser`.
- Earlier wave evidence (not re-rendered here): `ev_4bf4ac268a018a5e377584d729b63b1d10e6682cc1f1ee35fc153d3727099360`
  (EventKit overview, cited in `parity-matrix/data/eventkit.json`).

## Remaining gaps

- Calendar list grouping by source (EKSource): the sidebar shows one header ("On My Computer") and one local
  calendar. The app has no account model (no CalDAV or Exchange source), so there is nothing to group or to hide
  per source. Not wired.
- Multiple alarms per event: the imported file keeps several VALARM triggers (`IcsEvent::alarms`), but the event
  store holds one alert per event (`CalendarEvent::alert`, an index into a fixed list). The editor's Alert row
  still offers one choice. Changing that needs a store field, and the store is shared with the shell
  (`springboard/src/shell/calendar/store.rs`), which is outside this app. Not wired.
- Custom recurrence (every N days or weeks, end date or count): the editor's Repeat row cycles the store's
  fixed `REPEATS` list, and the same store limit applies (one `repeat` byte with an agreed meaning in the shell).
  The recurrence engine (`calendar/recur.rs`) already reads INTERVAL, COUNT and UNTIL from imported rules, so only
  the editor's choice is missing. Not wired.
- No import entry point: `Calendar::import_ics` has no caller in the app (see "How a user reaches it").
- Invitees cannot be edited, invited or answered from the app (the platform reference says the same). The row is
  read-only for events whose sidecar holds attendees; events created in the app have no attendee list.
- Invitee availability (busy, free, tentative) remains open (see `docs/parity/wire3-calendar.md`).
- The Invitees summary is one line; with long names it may run past the editor's width. Not measured on Midget.
- Event store access (`calendar_access.rs`), store change notifications (`calendar_store_changes.rs`) and errors
  (`calendar_error.rs`) are unchanged in this round; wire 4 and wire 5 notes still list them as open.
- Structured locations (`location_reminder.rs`): no geofence or structured location in the store. Not wired.

## Matrix rows

| Slug | Feature | BifrOSt path |
|---|---|---|
| `eventkit` | Participants | `springboard/ui/src/calendar_participant.rs`; `springboard/apps/calendar-desktop/src/calendar/invitees.rs` |
