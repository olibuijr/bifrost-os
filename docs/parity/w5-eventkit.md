# Wave 5: EventKit parity (pure Rust models)

Three rows of the EventKit framework (`docs/parity-matrix/data/eventkit.json`) now have a pure Rust model in
`springboard/ui`. None is wired into the shell or the apps yet, and none has been run on the VM or Midget. The event
store (`springboard/src/shell/calendar/store.rs`) is unchanged, and no SDK API was added, so the "Event store and
access" and "Events" rows stay `os` in spirit; only the access model is new.

## What was built

- **Participants** (`springboard/ui/src/calendar_participant.rs`, `Participant`). A read-only attendee: name, URL,
  role, type, status and whether it is the account owner. The type has no setters, because EventKit cannot add
  participants or change their information. `address()` drops a `mailto:` scheme; `display_name()` falls back to the
  address when the name is blank. The URL must have a scheme.
- **Calendar access** (`springboard/ui/src/calendar_access.rs`, `Access`). One answer each for events and reminders.
  Events take a write-only grant (add, never read) or a full grant; reminders take only a full grant. A first answer
  sticks: later requests return the current status and show no prompt. A system restriction overrides any answer.
  `can_read` is true only for full access, `can_write` for write-only and full access.
- **Virtual conferences** (`springboard/ui/src/calendar_conference.rs`, `Provider`). The provider holds room types
  (title and a provider-chosen identifier), join addresses, and one conference per room. `fetch_available_room_types`
  and `fetch_virtual_conference(room_id)` answer from that table. Duplicate room identifiers are refused, and a lookup
  distinguishes an unknown room from a room with no conference. Nothing opens a connection; the table is filled by the
  provider.

## Official API names mirrored

- EKParticipant (`name`, `url`, `isCurrentUser`, `participantRole`, `participantType`, `participantStatus`)
  https://developer.apple.com/documentation/eventkit/ekparticipant
- EKParticipantRole, EKParticipantType, EKParticipantStatus (the enum cases, as listed on their pages)
- EKAuthorizationStatus (`fullAccess`, `writeOnly`, `denied`, `notDetermined`, `restricted`; the deprecated
  `authorized` is left out). Request calls mirrored: `requestWriteOnlyAccessToEvents`, `requestFullAccessToEvents`,
  `requestFullAccessToReminders`, `authorizationStatus(for:)`.
  https://developer.apple.com/documentation/eventkit/ekauthorizationstatus
- EKVirtualConferenceProvider (`fetchAvailableRoomTypes(completionHandler:)`,
  `fetchVirtualConference(identifier:completionHandler:)`) https://developer.apple.com/documentation/eventkit/ekvirtualconferenceprovider
- EKVirtualConferenceRoomTypeDescriptor (`init(title:identifier:)`, `title`, `identifier`)
  https://developer.apple.com/documentation/eventkit/ekvirtualconferenceroomtypedescriptor
- EKVirtualConferenceDescriptor (`init(title:urlDescriptors:conferenceDetails:)`) and
  EKVirtualConferenceURLDescriptor (title and URL) https://developer.apple.com/documentation/eventkit/ekvirtualconferencedescriptor

Completion handlers are replaced by synchronous return values, since the models are pure state.

## REA evidence

- `ev_fe294a0d917822311c93020b0f23b79987d8b8fdc725ed7c6e75129c165eb34d` (EKParticipant page, rendered)
- `ev_97c387496b020b69ceb443943f62b437dcb1699b8b4e50bfb956edc05ff499fc` (EKAuthorizationStatus page, rendered)
- `ev_85520bc1771bfe3f538d60d69e469eeb0ddabd5ac7bab8458e59e96b97eb173a` (EKVirtualConferenceProvider page, rendered)
- `ev_cf6e58b96a171414b2987829c474c97d066de9fbf6237f98af01e57ad4e582fb` (EKVirtualConferenceRoomTypeDescriptor page, rendered)

DocC pages read with `tools/parity/docc.py`: `eventkit/ekparticipant`, `eventkit/ekparticipant/url`,
`eventkit/ekparticipant/iscurrentuser`, `eventkit/ekparticipant/participantstatus`, `eventkit/ekparticipantrole`,
`eventkit/ekparticipanttype`, `eventkit/ekparticipantstatus`, `eventkit/ekauthorizationstatus`,
`eventkit/ekvirtualconferenceprovider`, `eventkit/ekvirtualconferenceroomtypedescriptor`,
`eventkit/ekvirtualconferencedescriptor`.

## Matrix rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| EventKit | Participants | partial | springboard/ui/src/calendar_participant.rs |
| EventKit | Event store and access | partial | springboard/ui/src/calendar_access.rs |
| EventKit | Virtual conferences | partial | springboard/ui/src/calendar_conference.rs |

## Still open

- No app-facing SDK API: the access model has no shell wiring, so no prompt is shown and no app can call it yet.
- Participants are not read from the event store; the calendar's ICS model has no attendee list.
- The virtual conference table is not connected to any extension point, and the calendar does not offer rooms.
- The Apple requirement that a second, full-access request upgrades a write-only grant is not on the pages read here,
  so the model keeps the first answer and returns it unchanged.
- The raw values of the enums and the EventKit error domain are not on the pages read, so they are not modelled.

## Tests

`cargo` was not run (wave rule). Each new module has its own `#[cfg(test)]` block: 6 tests in
`calendar_participant.rs`, 6 in `calendar_access.rs`, and 7 in `calendar_conference.rs`. Expected values were worked
out by hand from the rules in the code.
