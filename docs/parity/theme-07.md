# Theme 07: Shortcuts app integration (parity wave 2)

## What was built

`springboard/ui/src/intent_resolve.rs` (module `bifrost_ui::intent_resolve`, one `pub mod` line in `springboard/ui/src/lib.rs`).
A pure model for typed actions, with no I/O and no Apple names:

- `ParamSpec` and `Kind` (`Text`, `Int`, `Bool`, `Enum`, `Entity` with `Candidate` ids and titles) describe the declared parameters.
- `resolve(specs, supplied, confirm)` checks the values the user gave, in declaration order, and returns one `Resolution`:
  `Ready` (resolved values), `Needs` (a required parameter is missing), `Disambiguate` (entity text matched several candidates,
  with their ids), `Invalid` (value does not fit its type) or `Confirm` (all values resolved and the action asks before it runs).
- Entity matching: an exact id wins; then an exact title (case-insensitive); then a single partial title match. Several exact or
  partial matches produce `Disambiguate`. An entity with no candidate list accepts any non-empty text.
- `Mode` and `pick_mode(supported, want_foreground)` choose the execution mode: the requested side when the action supports it,
  otherwise the first mode it declares, and `None` when it declares none.

Nothing in this module is wired into `intent.<id>` yet. The SDK still runs parameter checks through `Intent::validate` in
`springboard/sdk/src/intents.rs`. Serving `Resolution` over the control socket, and the confirmation dialog, are the next step.

## Apple API mirrored (official documentation)

- `AppIntent.supportedModes` (foreground and background modes the action supports): https://developer.apple.com/documentation/appintents/appintent/supportedmodes
- `IntentParameter` (input argument of an action): https://developer.apple.com/documentation/appintents/intentparameter
- `IntentResult` (result and dialog): https://developer.apple.com/documentation/appintents/intentresult
- SiriKit intent parameter resolution, confirmation and disambiguation: the matrix row's reference titles only. The Apple
  pages for resolution were not fetched in this round, so the behaviour is modelled from the row and not checked against them.

Not mirrored: `authenticationPolicy`, `allowedExecutionTargets`, `continueInForeground()` and the Shortcuts app itself.

## REA evidence

- `ev_9c9eb09306659b8e12d1a5fbbea6ac89929141f5c1bee1efc454bf7ea3c75746` (App Intents landing page, rendered)

## Matrix rows closed

| framework slug | feature (as in data/<slug>.json) | new status | BifrOSt path |
|---|---|---|---|
| siri-kit | Parameter resolution, confirmation and disambiguation | partial | springboard/ui/src/intent_resolve.rs (`resolve`, `Resolution`) |
| app-intents | Execution modes and authentication | partial | springboard/ui/src/intent_resolve.rs (`Mode`, `pick_mode`); no authentication policy |

Still open in this theme: `siri-kit` Shortcuts app integration (no Shortcuts app), `app-intents` Snippets and result views,
and the rest of the 37-row theme.

## Tests

`cargo` was not run (wave rule: no builds in this round). The module has 9 `#[cfg(test)]` behaviour tests in
`intent_resolve.rs`: missing required input, exact and title matching, partial-match disambiguation, type checks,
enum case matching, optional values and confirmation order, entity without candidates, and mode selection.
`python3 tools/check-names.py` passes (exit 0).
