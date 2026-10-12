# Wave 4, theme-10: permission questions and answers

Theme: "Creating a communication experience (AskCenter, PermissionQuestion, AskPermissionAction, AskError)", 19 gap
rows across 10 frameworks. This round builds the request-and-answer flow of the communication-permission center.
The other rows of the theme (photo library, StoreKit, CallKit, EventKit and so on) are not touched; they belong to
their own areas.

## What was built

`springboard/ui/src/permission_question.rs` (bifrost-ui, pure model, host unit tests, one `pub mod` line in
`springboard/ui/src/lib.rs`). No new crates, no unsafe, no hardware, no network.

- `QuestionTopic` trait with two topics: `CommunicationTopic` ("communication") and `SignificantAppUpdateTopic`
  ("significant-app-update").
- `PermissionQuestion<T>`: a question with an id, a topic, choice ids, a default choice and an optional expiry.
  `communication(id, handles, expires_at)` takes 1 to 32 contacts (`Handle`: non-blank, at most 256 bytes, no control
  characters). `significant_app_update(id, expires_at)` takes no contacts. `with_choices` replaces the choices
  (1 to 8 unique, non-blank ids, a valid default index). The default choices are "approve" and "decline" with
  "decline" as the default. This default is a BifrOSt choice; the system defines the real set on Apple's side.
- `AskError`: `Unknown`, `CommunicationLimitsNotEnabled`, `ContactSyncNotSetup`, `InvalidQuestion`,
  `SystemError(String)`, `NotAvailable`, with an English `description()`.
- `AskCenter`: the queue. `ask` sends a question (refused when expired, duplicate, or when 16 are waiting). `answer`
  records a choice (refused when unknown, expired or not one of the offered choices; the question keeps waiting for a
  wrong choice). `expire(now)` drops waiting questions past their deadline. `take_responses(topic)` drains the answers
  for one topic in arrival order and keeps the others. `is_pending`, `pending_count`, `answered_count` report state.
- `PermissionResponse`: question id, topic name, chosen id and the time it was answered.

Time and ids come from the caller (seconds, and a `u64` standing in for the question UUID). The model reads no clock.

## Official API names mirrored

From the DocC pages of PermissionKit (`python3 -I tools/parity/docc.py permissionkit/...`):

- `AskCenter` (`ask`, `responses(for:)`, `shared`)
- `PermissionQuestion` (`init(handle:)`, `init(handles:)`, `init(communicationTopic:)`,
  `init(significantAppUpdateTopic:)`, `choices`, `defaultChoice`, `id`, `topic`, `expirationDate`)
- `AskPermissionAction` (`callAsFunction(_:)`), mirrored as `AskCenter::ask`. The model has no environment value; the
  action is the same send call.
- `AskError` (`unknown`, `communicationLimitsNotEnabled`, `contactSyncNotSetup`, `invalidQuestion`, `systemError`,
  `notAvailable`, `errorDescription`)
- `QuestionTopic` (protocol)
- `PermissionResponse` (the answer carried back by `responses(for:)`)

## REA evidence (official rendered pages, Chromium via `tools/parity/rea-page.sh`)

| Page | REA evidence id |
|---|---|
| developer.apple.com/documentation/permissionkit/askcenter | ev_a7ac08f4a90abe84258524f615749dd5fd0a0dd3d76eedc60df19faf009d409e |
| developer.apple.com/documentation/permissionkit/permissionquestion | ev_29086451b7256d07a4c3c0780bec9948869de2631657db27d9113e4108247ba5 |
| developer.apple.com/documentation/permissionkit/askerror | ev_af7000d863eb951cf78172f014b5b0e0108883b8f75ae033dd92db3e319e7c80 |
| developer.apple.com/documentation/permissionkit/askpermissionaction | ev_a8ba4a60350360e9eb274a030449eb3808a60aee4f6776eaf1b17406f514ee53 |

## Matrix rows closed

Status is partial: the model is pure and has host tests. It is not wired into the SDK or the shell, there is no
guardian channel, and it has not been run on the VM or Midget.

| Framework slug | Feature (as in docs/parity-matrix/data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| permissionkit | Creating a communication experience (AskCenter, PermissionQuestion, AskPermissionAction, AskError) | partial | springboard/ui/src/permission_question.rs |
| permissionkit | Permission responses stream (responses(for:), PermissionResponse, PermissionChoice) | partial | springboard/ui/src/permission_question.rs (AskCenter::take_responses, PermissionResponse) |
| permissionkit | Question topic protocol (QuestionTopic) | partial | springboard/ui/src/permission_question.rs (QuestionTopic, CommunicationTopic, SignificantAppUpdateTopic) |

## Not closed here

- Delivery to a guardian and the SDK entry point (`springboard/sdk/src/`) are not built. The control socket is
  fire-and-forget, so there is no response channel yet.
- The "PermissionChoice" and "PermissionButton" presentation are not built.
- Other theme-10 rows (PhotoKit, StoreKit, CallKit, EventKit, Security Foundation, CloudKit, Background Assets) belong
  to other areas and are untouched.
