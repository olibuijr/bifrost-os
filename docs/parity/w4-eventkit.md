# Wave 4: EventKit parity (pure Rust models)

Three gap rows of the EventKit framework (`docs/parity-matrix/data/eventkit.json`) now have a pure Rust model in
`springboard/ui`. None of them is wired into the shell or the apps yet, and none has been run on the VM or Midget.
The event store itself (`springboard/src/shell/calendar/store.rs`) is unchanged.

## What was built

- **Change notices** (`springboard/ui/src/calendar_store_changes.rs`, `StoreChangeHub`). A store calls `post()` after
  each save. Observers `subscribe()`, then `acknowledge()` to learn whether anything changed since they last looked.
  Many saves between two checks give one pending refetch. Observers do not replay earlier changes, and their ids are
  not reused. It mirrors the change notification that EventKit posts when the calendar database changes.
- **Location-based reminders** (`springboard/ui/src/geofence.rs`, extended). Adds `StructuredLocation` (title, centre,
  trigger radius; mirrors the title, geoLocation and radius of the structured location class), `Proximity`
  (enter, leave or both, set with `CircularRegion::with_proximity`) and `RegionMonitor::add_location`. The alarm
  reuses the existing region monitor, so the accuracy rule and state handling are the same as for any region.
- **Error codes** (`springboard/ui/src/calendar_error.rs`, `ErrorCode`). The 36 codes of the official error list (the
  internal `last` marker is left out), a `Category` (access, validation, read-only, unsupported, internal) for each,
  and a one-sentence `Display` taken from the official descriptions.

## Official API names mirrored

- EKEventStoreChangedNotification (change notice; doc: "A notification posted when changes are made to the Calendar database.")
- EKStructuredLocation (`init(title:)`, `title`, `geoLocation`, `radius`)
- EKError.Code (the 36 codes; `EKErrorDomain` is not modelled, because the domain string value is not on the page)
- Official overview: https://developer.apple.com/documentation/eventkit

## REA evidence

- `ev_ac0eaaa43821b001cddf5a191bc2a92780b0eeea81f8a1868e26ab1de4c08cf6` (EventKit landing page, rendered, this wave)
- `ev_4bf4ac268a018a5e377584d729b63b1d10e6682cc1f1ee35fc153d3727099360` (matrix row evidence, wave 1)

DocC pages read with `tools/parity/docc.py`: `eventkit`, `eventkit/ekerror`,
`eventkit/ekeventstorechangednotification`, `eventkit/ekstructuredlocation`.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| EventKit | Change notifications | partial | springboard/ui/src/calendar_store_changes.rs |
| EventKit | Location-based reminders | partial | springboard/ui/src/geofence.rs (StructuredLocation, RegionMonitor::add_location) |
| EventKit | Errors | partial | springboard/ui/src/calendar_error.rs |

## Still open

- No app-facing SDK API for events or reminders (the "Event store and access" and "Events" rows stay `os`).
- Change notices are not delivered to other apps yet: the shell still polls the store generation.
- No participants, account sources, virtual conference extension or proximity alarms in the store model.
- The official `EKErrorDomain` string value is not modelled.

## Tests

`cargo` was not run (wave rule). The new and changed modules carry their own `#[cfg(test)]` blocks: 6 tests in
`calendar_store_changes.rs`, 3 added to `geofence.rs`, and 5 in `calendar_error.rs`. Expected values were worked out by hand from the rules in the code.
