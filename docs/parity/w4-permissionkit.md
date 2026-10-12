# PermissionKit: parent or guardian permission requests (wave 4)

Status: model built, not wired into the shell or the App Store flow. Not compiled or tested in this round: the
round ran with no builds (see "Verification").

## What was built

A pure model of the request side of PermissionKit: an app sends a permission question to a parent or guardian,
the guardian answers, and the app drains the answer per topic.

- `springboard/core/src/permission_request.rs` (`bifrost_core::permission_request`), 8 behaviour tests.
  - `Availability`: the three switches the center checks (service, communication limits, contact sync).
  - `Choice`, `Answer`: the selectable answers; `Choice::approve()` and `Choice::decline()` are the standard pair.
  - `Topic` trait (the question topic protocol): `topic_id()` and an optional `handles()`.
    `SignificantAppUpdate` and `Communication` implement it.
  - `Question`: built with `new` or `yes_no`, with `with_default` and `with_expiration`. The default is decline.
  - `Center`: `ask` (checks switches, then question shape, then issues a number), `respond`, `expire`,
    `responses_for(topic_id)`, `pending`, `set_availability`.
  - `Response`: the original question and the chosen choice.
  - `Error`: the documented cases plus `UnknownQuestion`, `UnknownChoice`, `Expired` (needed by the model).
- `springboard/sdk/src/permission_kit.rs` (`bifrost_app::permission_kit`): re-exports the surface above so an app
  imports one path. No new logic in the SDK layer.
- Module lines: one `pub mod permission_request;` in `springboard/core/src/lib.rs` and one
  `pub mod permission_kit;` in `springboard/sdk/src/lib.rs`.

Time is passed in as whole seconds. The model has no clock, bus or thread, so it needs no hardware or network.

## Official API names mirrored

Source: developer.apple.com/documentation/permissionkit and its symbol pages (read with `tools/parity/docc.py`).

| Official name | BifrOSt name |
|---|---|
| `AskCenter` (`ask(_:in:)`, `shared`) | `Center::ask` (no shared instance; the caller owns the center) |
| `PermissionQuestion` (`init(communicationTopic:)`, `init(significantAppUpdateTopic:)`, `choices`, `defaultChoice`, `id`, `topic`, `expirationDate`) | `Question::new`, `Question::yes_no`, `choices`, `default_choice`, `id`, `topic_id`, `expires_at` |
| `SignificantAppUpdateTopic` | `SignificantAppUpdate` |
| `CommunicationTopic` | `Communication` (handles are plain strings; no `CommunicationHandle` type) |
| `QuestionTopic` (`id`) | `Topic::topic_id` |
| `responses(for:)` | `Center::responses_for(topic_id)` (drains a `Vec`, not an async sequence) |
| `PermissionResponse` (`question`, `choice`) | `Response::question`, `Response::choice` |
| `PermissionChoice` (`answer`, `approve`, `decline`, `id`, `title`) | `Choice` (`answer`, `approve`, `decline`, `id`, `title`) with `Answer` |
| `AskError` (`unknown`, `communicationLimitsNotEnabled`, `contactSyncNotSetup`, `invalidQuestion`, `notAvailable`, `errorDescription`) | `Error` and `Error::description` |

## REA evidence

- Landing page (PermissionKit overview, read through `tools/parity/rea-page.sh`):
  `ev_38c2d0f0f21d9d075f691b70ca551b26595dac3b3dff8f8e920f2e38ca957b94`.
  It confirms the topic list (AskCenter, PermissionQuestion, the two topics, PermissionButton, responses(for:),
  PermissionResponse, PermissionChoice, QuestionTopic, AskError, PermissionFlow, PermissionResult) and the note that
  communication experiences are available only through the messaging service.
- The matrix's existing evidence for the framework is `ev_cd9fa96eaa8c651fdce9299018cdf03ab50f910d59b0e3e092d23a839bec79fe`.

## Rows closed

| Framework | Feature (as in `docs/parity-matrix/data/permissionkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| PermissionKit | Creating a communication experience (AskCenter, PermissionQuestion, AskPermissionAction, AskError) | partial | springboard/core/src/permission_request.rs, springboard/sdk/src/permission_kit.rs |
| PermissionKit | Permission responses stream (responses(for:), PermissionResponse, PermissionChoice) | partial | springboard/core/src/permission_request.rs, springboard/sdk/src/permission_kit.rs |
| PermissionKit | Question topic protocol (QuestionTopic) | partial | springboard/core/src/permission_request.rs, springboard/sdk/src/permission_kit.rs |
| PermissionKit | Significant app update topic | partial | springboard/core/src/permission_request.rs, springboard/sdk/src/permission_kit.rs |

`partial` rather than `parity`: the model has no UI, no delivery to a real guardian, and no link to the Screen
Time shell. The `Communication` topic has no contact or handle type. `Permission button UI` (PermissionButton) and
`Communication limits` stay as they are in the matrix: no SDK button was built, and the communication limits
are still the shell's per-app limits.

## Not built

- `PermissionButton`: a sheet or button that presents the guardian flow. It needs the shell UI and the Screen Time
  shield; left as a gap.
- Delivery: nothing sends the question to a guardian's device or receives the answer from one. The shell does not
  call `Center` yet.
- Expiry: `expire` must be called by the owner; no timer runs.

## Verification

No builds or tests were run, per the round's rules (no cargo, rustc or make). The code was written against the
neighbour modules' idioms and hand-checked: integer subtraction uses `before - len` only after `retain` (cannot
underflow), `respond` removes from `pending` only after checks pass, and comparisons between `String` and `&str`
use the standard impls. `python3 tools/check-names.py` reports no hits for the new files. The index call
(`code_search`) timed out, so repo lookups were done with Grep after that one attempt.

Test list (`springboard/core/src/permission_request.rs`, `mod tests`):
1. `yes_no_defaults_to_decline_and_numbers_increase`
2. `malformed_questions_are_rejected`
3. `switches_block_questions_in_order`
4. `approve_moves_the_question_to_responses`
5. `unknown_choice_keeps_the_question_pending`
6. `expiry_blocks_answers_and_expire_drops_questions`
7. `responses_are_drained_per_topic`
8. `disabled_center_drops_no_state`

Eight tests are present, all with expected values worked out from the rules above.
