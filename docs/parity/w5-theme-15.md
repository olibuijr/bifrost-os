# Wave 5, theme 15: virtual conferences

Branch `parity/w5-theme-15`. Pure Rust, std only, no new crates. Nothing was built or run: the
tests were written from hand-worked expectations and have not been executed (the task forbids builds).

## What was built

- `springboard/ui/src/conference.rs` (new, `bifrost_ui::conference`). Two parts:
  - `Roster` of `Participant`s (person, group, room or resource; role; status; the account owner flag).
    Addresses must have a scheme (`mailto:`, `https:`); duplicates are refused without regard to case.
    `Roster::attendance()` counts the answers of people and groups only. Rooms, resources and
    non-participants are left out, as they do not answer an invitation.
  - `ConferenceProvider` with `RoomType`s (room identifier and title) and one `Conference` per room
    (title, one or more `JoinLink`s, optional details). Join links must be `https://`. Setting a
    conference needs an offered room; fetching one reports an unknown room or an unset conference.
- `springboard/ui/src/call.rs` (extended). `StartCallAction` (handle, `is_video`, `contact_identifier`)
  and `CallCenter::start_action`, which starts a dialing outgoing call. `Call` gains `video` and
  `contact_identifier`, which other call paths leave `false` and `None`.
- `springboard/ui/src/lib.rs`: one `pub mod conference;` line.

Limits: the models are not wired into the calendar store, the shell or any app. The start action
uses the call's numeric id, not the caller-chosen UUID that the official action takes.

## Official API names mirrored

- EventKit: `EKVirtualConferenceProvider` (`fetchAvailableRoomTypes`, `fetchVirtualConference(identifier:)`),
  `EKVirtualConferenceRoomTypeDescriptor` (`title`, `identifier`), `EKVirtualConferenceDescriptor`
  (`title`, `urlDescriptors`, `conferenceDetails`), `EKParticipant` (`name`, `url`, `isCurrentUser`,
  `participantRole`, `participantStatus`, `participantType`), and the enums `EKParticipantType`,
  `EKParticipantRole`, `EKParticipantStatus` with the case lists from their official pages.
- CallKit: `CXStartCallAction` (`handle`, `isVideo`, `contactIdentifier`).

Our rule, not Apple's: a conference needs at least one join link, and join links must be `https://`.
Apple's pages do not state either rule in these terms.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/eventkit/ekvirtualconferenceprovider | ev_54c9633c585669945eb80d3031803867af26c2d6580a039df8140d329019ec57 |
| https://developer.apple.com/documentation/eventkit/ekparticipant | ev_553267ded114bb7068c7dc6bf3fb8dedcdea67d138ca542715caaf7159b2e32e |
| https://developer.apple.com/documentation/callkit/cxstartcallaction | ev_0bad2770fe2b3fbea3edc137729b4f589c1ba97593695e56c124ea0b9e1b1a4b |

The enum and descriptor pages were read with `tools/parity/docc.py` only (no REA run).

## Matrix rows closed

| Framework slug | Feature (as in data file) | New status | BifrOSt path |
|---|---|---|---|
| eventkit | Participants | partial | springboard/ui/src/conference.rs |
| eventkit | Virtual conferences | partial | springboard/ui/src/conference.rs |
| callkit | Outgoing calls | partial | springboard/ui/src/call.rs |

## Not closed in this theme

- storekit "StoreKit testing": no StoreKit test surface was built.
- screencapturekit "System content-sharing picker ...": the picker row needs a portal frontend, out of scope here.
- relevancekit "Time clues" and extensionfoundation "Adding support for app extensions ...": not touched.
