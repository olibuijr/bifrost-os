# CallKit parity

Official reference: https://developer.apple.com/documentation/callkit (macOS 13.0 and later; the page also lists
iOS and Mac Catalyst).

REA evidence of the rendered landing page: `ev_e21f40d7e241ade388ee21096b91e0407c0892f83fc4d5473f2448be88498c3c`.
The matrix row before this change cited `ev_c384112764dab667ab3b20bceceb0c43a831cada06b9a38eabf6e189b83c4dc9`.

## What was built

`springboard/ui/src/call.rs` (`bifrost_ui::call`), pure std Rust, no shell or audio dependency:

- `Handle`: a phone number, an email address or a generic handle, with `validate()`.
- `CallState` and `Call`: dialing, ringing, connected, held. A call that ends leaves the list.
- `CallAction`: answer, end, set held, set muted, play DTMF (digits `0`-`9`, `*`, `#`, `A`-`D`).
- `CallCenter`: `start_outgoing`, `report_incoming`, `report_connected`, `perform`, `perform_transaction`,
  `calls`, `call`, `take_events`. Transactions are all-or-nothing. Events are `Added`, `Changed` and `Ended`,
  and `Changed` is sent only when a call differs.

Seven tests in `call.rs` cover handle validation, the outgoing lifecycle, incoming answer and hold, mute before
answer, transaction atomicity, DTMF symbols and state checks, and removal of one ended call.

## Official API names mirrored

| Official | Here |
|---|---|
| CXProvider, CXProviderDelegate | `CallCenter` (the provider side; no delegate yet) |
| CXCallUpdate, CXAnswerCallAction | `report_incoming`, `CallAction::Answer` |
| CXCallController, CXTransaction | `CallCenter::perform`, `perform_transaction` |
| CXStartCallAction | `start_outgoing` |
| CXEndCallAction, CXSetHeldCallAction, CXSetMutedCallAction, CXPlayDTMFCallAction | `CallAction::End`, `SetHeld`, `SetMuted`, `PlayDtmf` |
| CXCall, CXCallObserver, CXHandle | `Call`, `CallCenter::calls` and `take_events`, `Handle` |

## Status

Rows in `docs/parity-matrix/data/callkit.json` are now `partial` (provider and state, incoming, outgoing, call
actions, observation). Caller ID and blocking and the default calling app stay `gap`. The framework status is
`partial`.

## Remaining gaps

- Provider registration and configuration reachable from apps through the SDK.
- A call screen or banner in the shell, plus push delivery for incoming calls.
- Audio routing and DTMF tone output (belongs to `springboard/audiod`).
- Group and translate call actions.
- Caller ID and blocking directory extension.
- Default calling app setting.

## Verification

Not built or run in this stage, per the stage rules. The name guard `python3 tools/check-names.py` passes.

## Incoming call UI row (review of parity/team-a-03)

The branch first added a second incoming-call prompt to `call.rs`. It duplicated `springboard/ui/src/call_ui.rs`
(`CallUpdate`, `IncomingCard`, `presentation`), which already holds the banner and full-screen card with answer and
decline requests, so the duplicate was removed. The "Incoming call UI" row now cites `call_ui.rs` and is `partial`: the
shell draws no call card and the row is not verified on Midget.
