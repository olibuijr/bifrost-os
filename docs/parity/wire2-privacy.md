# Wire 2: Privacy & Security (location authorization)

Branch `wire2/privacy`, based on main 0.5.37. Scope: the location authorization model
(`springboard/ui/src/location_auth.rs`) is now wired through the existing `privacy` service and shown in
Settings > Privacy > App Permissions. The permission question and authorization request models are not wired (see gaps).

## What is now wired

| Piece | Where | What a user or app gets |
|---|---|---|
| Per-app location status | `springboard/src/shell/services/privacy_permissions.rs` (`location_status`, `set_location_by_user`) | A thread-local store of `AuthorizationStatus` per app id. A new app is `not_determined`. |
| Settings switch | `springboard/src/shell/pages.rs` (`permissions_page`, `Toggle::Location`), `springboard/src/shell/settings.rs` (toggle arm) | A LOCATION group under App Permissions, one switch per installed app. On means `authorized_when_in_use`; off means `denied`. A `restricted` app keeps its status. |
| Service method `location_status` | `privacy_permissions.rs` `answer` | `{status, authorized}`. |
| Service method `location_request` | `privacy_permissions.rs` `answer` | `{outcome}`: `prompt`, `already_granted` or `refused`, from `AuthorizationStatus::request_outcome`. |
| SDK client | `springboard/sdk/src/privacy_permissions.rs` (`location_status`, `location_request`, `decode_location`, `decode_location_outcome`) | Typed calls and pure decoders for the replies, with host tests. |

The privacy service and the wire protocol are unchanged in shape. The new methods are additional names on the
existing `privacy` service channel. `springboard/proto` is not touched.

## How a user reaches it

Settings > Privacy & Security > App Permissions. The LOCATION group sits under the camera, microphone and screen
recording groups. Each installed app has a switch, and the footer says a switch is on only when the person allows it.

## How to verify on Midget

1. Recover and unlock Midget as the bifrost-pc skill describes (`bifrost pc midget state`, `bifrost pc midget key enter`).
2. Open Settings and go to Privacy & Security > App Permissions (`bifrost pc midget tap-window Settings X Y` to reach the row).
3. Confirm a LOCATION group appears after SCREEN RECORDING, with one switch per installed app, all off for a new app.
4. Turn an app's switch on and confirm it reads on. Turn it off and confirm it reads off.
5. Restart the shell and confirm the answers reset. This is expected: nothing is written to disk yet (see gaps).
6. From an app that calls `location_status` through the SDK client, check `authorized` matches the switch.

Host tests (not run in this round, lead to build): `privacy_permissions` (location tests), `pages::tests`
(`app_permissions_lists_each_media_type_with_one_switch_per_app`), and `sdk privacy_permissions` (location decode tests).

## REA evidence

- `ev_d7e646c86260cc640d440716084aed24598e88ec5de0666fef0a2eec9959cf97`: official page CLAuthorizationStatus
  (`https://developer.apple.com/documentation/corelocation/clauthorizationstatus`). Shows the five statuses and the
  `notDetermined` and `denied` behaviour.
- `ev_0b8168e5d5d679a14e2f408dc4e2c4b35c4bb95f5cf065d32ad36deed6d85a9f`: official page PermissionButton
  (`https://developer.apple.com/documentation/permissionkit/permissionbutton`). Used to scope the permission-question gap.

The official "Requesting authorization to use location services" article was read with `docc.py`
(`corelocation/requesting-authorization-to-use-location-services`), not with REA.

## Matrix rows

| Slug | Feature (as written in the matrix json) | BifrOSt path | Status after this wave |
|---|---|---|---|
| core-location | Authorization and usage descriptions | springboard/ui/src/location_auth.rs (wired: status, request outcome, Settings switch, service methods) | partial. Usage descriptions are still missing. |
| core-location | Errors and diagnostics | springboard/ui/src/location_auth.rs | still partial and not wired. It depends on a location manager, which does not exist. |
| permissionkit | Permission button UI (PermissionButton) | springboard/ui/src/permission_question.rs, springboard/sdk/src/permission_kit.rs | not wired (gap) |
| permissionkit | Creating a communication experience (AskCenter, PermissionQuestion, AskPermissionAction, AskError) | springboard/ui/src/permission_question.rs | not wired (gap) |
| permissionkit | Permission responses stream (responses(for:), PermissionResponse, PermissionChoice) | springboard/ui/src/permission_question.rs | not wired (gap) |
| permissionkit | Question topic protocol (QuestionTopic) | springboard/ui/src/permission_question.rs | not wired (gap) |
| authentication-services | Authorization requests (controller, results) | springboard/ui/src/authorization_request.rs | not wired (gap) |

## Remaining gaps

- No prompt. A `location_request` that returns `prompt` does not show a sheet. The person answers in the Settings switch.
  Adding a sheet needs a shell-side request queue and is a separate round.
- No usage descriptions. The `NSLocation*UsageDescription` texts are not part of the manifest, so the model's
  `UsageDescriptions` is not used.
- No manifest gate. The manifest permission list (`network, storage, audio, notifications, documents, keychain,
  remote-desktop`) has no location entry. The location store is not checked against the manifest, the same as the
  capture store.
- No accuracy. `AccuracyAuthorization` is not stored or reported. A switch on means `authorized_when_in_use` with no
  reduced-accuracy choice.
- No location manager. Nothing starts location services, so `LocationError` and diagnostics stay unwired.
- No persistence. Answers reset on shell restart, like the capture answers (docs/parity/wire-svc-privacy.md).
- Permission question (guardian approval) and authorization request (sign-in controller) models are not wired.
  Both need a request channel and a shell UI that the shell does not have yet.
