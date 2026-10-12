# Wire 3: Permission sheets (privacy requests)

Branch `wire3/permission-sheets`, based on main 0.5.38. Scope: when an app asks for location, the camera, the
microphone or screen recording through the `privacy` service, the shell now shows an Allow / Don't Allow sheet. The
sheet is built from the permission question model (`bifrost_ui::permission_question`). The wire protocol
(`springboard/proto`) is not touched.

## What is now wired

| Piece | Where | What a user or app gets |
|---|---|---|
| Permission sheet | `springboard/src/shell/permission_sheet.rs` (`Sheet`, `PrivacyTopic`, `Button`) | An alert card: the title "“App” would like to use your location." (or camera, microphone, record your screen), a short detail with the Settings path, and two buttons: Don't Allow, then Allow (bold, the default). |
| Question model | `Sheet::new` sends a `PermissionQuestion` for a privacy topic to its own `AskCenter` with choices `allow` and `dont_allow`; the default is `dont_allow`. `Sheet::finish` takes the answer and writes it. | An unanswered sheet never grants. |
| Question constructor | `springboard/ui/src/permission_question.rs` `PermissionQuestion::for_topic` | Any topic can build a question with the default choices (small addition; tests in `for_topic_tests`). |
| Request queue | `springboard/src/shell/services/privacy_permissions.rs` (`queue_prompt`, `take_prompt`, `still_asked`, `decide`, `wants_prompt`) | Requests wait in a thread-local queue (at most 8). The shell takes one at a time when no sheet is up. A request answered in Settings before its turn is dropped. |
| Service method `request` | `privacy_permissions.rs` `answer` and `call` | `request {media}` returns `{media, status}`. A `not_determined` media type queues the sheet. |
| Service method `location_request` | same | A `prompt` outcome now queues the location sheet (wire 2 had no sheet). |
| Shell hooks | `springboard/src/shell/mod.rs`: `permission_sheet` field, `permission_tick` (from the frame tick), touches in `modal_event`, draw in the modal block, the quiet-frame checks | The sheet sits above the windows and takes every touch until answered. |
| SDK client | `springboard/sdk/src/privacy_permissions.rs` `request(ctx, media)` | Apps ask for a media type; the reply decodes with `decode_status`. |
| SDK re-exports | `springboard/sdk/src/permission_kit.rs` | `AskCenter`, `AskError`, `PermissionQuestion`, `PermissionResponse`, `QuestionTopic` for apps (the same model the sheet uses). |

Answers are written to the same store as the Settings switches (`set_location_by_user`, `set_by_user`), so
Settings > Privacy & Security > App Permissions shows the answer straight away.

## How a user reaches it

An app calls `privacy_permissions::location_request` (or `request` for a media type) and the sheet appears on
the desktop. Nothing in Settings opens it; it is requested by an app.

## How to verify on Midget

1. Recover and unlock Midget (`bifrost pc midget state`, `bifrost pc midget key enter`).
2. Install or open an app that calls the SDK `request` or `location_request` for a media type it has not been
   answered for (for example `bifrost pc midget open-app ID`), or call the service from a test app.
3. Confirm the sheet appears above the windows with the app name in the title, and two buttons: Don't Allow and Allow.
4. Tap Allow (`bifrost pc midget tap-window APP X Y` with window-local pixels of the sheet's button, or `tap` for
   the shell geometry). Open Settings > Privacy & Security > App Permissions and confirm the app's switch is on.
5. Repeat with Don't Allow and confirm the switch is off. Confirm no second sheet comes for the same request.
6. Call the request again after an answer: no sheet should appear (the answer stands).

Host tests (lead to build): `privacy_permissions` (`a_prompt_is_wanted...`, `a_queued_request...`,
`a_media_sheet_that_is_denied...`, `a_request_answered_in_settings...`), `shell::permission_sheet` tests,
`ui permission_question::for_topic_tests`, `sdk privacy_permissions` (the request name test).

## REA evidence

- `ev_bfc6252f57ad41f28b92a8d9f0ecd9bcff6de71946ecfae41fe03c7acefde20c`: official page AskCenter
  (`https://developer.apple.com/documentation/permissionkit/askcenter`). Shows `ask` and `responses(for:)` and
  that the flow routes through parents or guardians.
- `ev_34d0d5d753ebfabd38522d896166bec1bc76e53d4a05caaa0d01ef8198828a8f`: official article "Requesting
  authorization to use location services"
  (`https://developer.apple.com/documentation/corelocation/requesting-authorization-to-use-location-services`).
  Shows that the system prompts the device owner once, then stores the status and does not prompt again. This is the
  basis for the `prompt` outcome queueing one sheet and later requests reading the stored answer.
- Also read with `docc.py` (no REA): `permissionkit/askcenter`, `corelocation/requesting-authorization-to-use-location-services`.

## Remaining gaps

- The sheet is the privacy prompt only. The PermissionKit guardian flow (AskCenter sending to a parent, the response
  stream from a parent, communication limits) is not a shell sheet; the models stay unwired by the shell, and apps
  get the types through the SDK re-exports only. No parent's answer is ever delivered.
- Authorization requests (`springboard/ui/src/authorization_request.rs`, the sign-in controller) are not wired. That
  flow needs a sign-in controller and a result channel the shell does not have. Not started in this round.
- `request` only queues a sheet. The app learns the answer by polling `status` or `location_status`. No reply is
  pushed when the person answers.
- No usage descriptions, no manifest gate and no accuracy choice (as in wire 2).
- Answers reset when the shell restarts (as in wire 2 and wire-svc-privacy).
- The sheet does not expire, and it is not dismissed by Escape or by a tap outside it. The person must pick a button.
- Not tested on Midget yet; the sheet was built for the host tests and the lead's build.

## Matrix rows

| Slug | Feature (as written in the matrix json) | BifrOSt path | Status after this wave |
|---|---|---|---|
| permissionkit | Creating a communication experience (AskCenter, PermissionQuestion, AskPermissionAction, AskError) | springboard/ui/src/permission_question.rs (`for_topic`), springboard/src/shell/permission_sheet.rs, springboard/sdk/src/permission_kit.rs | partial. The privacy sheet uses the question model; the guardian communication flow is still not wired. |
| permissionkit | Permission responses stream (responses(for:), PermissionResponse, PermissionChoice) | springboard/src/shell/permission_sheet.rs (`Sheet::finish` reads the response) | partial. The answer comes back to the shell only. No stream to apps. |
| permissionkit | Question topic protocol (QuestionTopic) | springboard/src/shell/permission_sheet.rs (`PrivacyTopic`) | partial. The shell implements the topic for privacy questions. |
| permissionkit | Permission button UI (PermissionButton) | springboard/ui/src/permission_question.rs | not wired (gap, unchanged). |
| core-location | Authorization and usage descriptions | springboard/src/shell/services/privacy_permissions.rs (`location_request` queues the sheet) | partial. The sheet shows the prompt; usage descriptions are still missing. |
| authentication-services | Authorization requests (controller, results) | springboard/ui/src/authorization_request.rs | not wired (gap). |
