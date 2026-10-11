# SiriKit parity: parameter resolution and intent donation

Wave 2 of the Apple macOS 27 parity matrix (`docs/parity-matrix/data/siri-kit.json`). Two gaps from that matrix
are now built as pure Rust models in the SDK. Neither needs a voice service, a network service or hardware, so the
voice path stays out of scope.

## What was built

- `springboard/sdk/src/intent_resolve.rs` (`bifrost_app::intent_resolve`): the steps that run before an intent is
  performed.
  - `resolve_param(param, value, choices)` returns a `Resolution`: `Success`, `NotRequired`, `NeedsValue`,
    `Unsupported(reason)` or `Disambiguate(options)`. An entity parameter with one matching choice resolves to that
    choice's id; with several choices it asks the user to pick.
  - `plan(intent, params, choices, needs_confirmation)` returns a `Plan`: `Ask`, `Choose`, `Blocked`, `Confirm` or
    `Ready`. It walks the parameters in declaration order and stops at the first one that is not settled. Unknown
    parameter names and a non-object request are blocked with an empty `param`.
  - The caller asks the question (through the shell or a test) and calls `plan` again with the answer. The module
    never talks to the user.
- `springboard/sdk/src/donation.rs` (`bifrost_app::donation`): the donation store that feeds Search suggestions.
  - `DonationStore::donate` records one completed action. A donation with the same persistent identifier replaces the
    earlier one. The store keeps the newest `limit` donations and drops the oldest.
  - `delete(identifiers)`, `delete_intent(intent_id)` and `delete_all()` return how many were removed.
  - `suggestions(k)` ranks donated actions by count, then by the most recent donation, then by title. Donations of
    one intent with different parameters are separate suggestions.
- Both modules are registered with one `pub mod` line each in `springboard/sdk/src/lib.rs`.

Tests: `intent_resolve` has 6 tests and `donation` has 6 tests (behaviour, not only construction).

## Official API names mirrored

- Resolving the parameters of an intent: resolution results (success, needs a value, unsupported, not required,
  disambiguation). `Resolution`, `resolve_param`.
  https://developer.apple.com/documentation/sirikit/resolving-the-parameters-of-an-intent
- Confirming the details of an intent: confirmation before performing. `Plan::Confirm`.
  https://developer.apple.com/documentation/sirikit/confirming-the-details-of-an-intent
- Donating shortcuts: one donation per completed action, with a persistent identifier. `DonationStore::donate`.
  https://developer.apple.com/documentation/sirikit/donating-shortcuts
- Deleting donated shortcuts: removing donations by identifier and all at once. `delete`, `delete_intent`,
  `delete_all`.
  https://developer.apple.com/documentation/sirikit/deleting-donated-shortcuts

## REA evidence

- `ev_7af96f1400dadad4d9ec6d762fa6b2579ea4ef6874e26748c440ea0945489f25`: Resolving the Parameters of an Intent
  (rendered page, title and overview text).
- `ev_885b7377830ce74475fe122c9d3d1ecb2f312ee6ecca10be824eeecf1a9d903d`: Donating Shortcuts (rendered page).

The confirmation and deletion articles were read through the DocC JSON only (`tools/parity/docc.py`), not through
REA.

## Matrix rows closed

| framework slug | feature (as in data/siri-kit.json) | new status | BifrOSt path |
|---|---|---|---|
| siri-kit | Parameter resolution, confirmation and disambiguation | partial | springboard/sdk/src/intent_resolve.rs |
| siri-kit | Donating interactions and shortcuts | partial | springboard/sdk/src/donation.rs |

Still open in this area: no voice input or spoken dialog, no custom vocabulary or synonyms, no shell surface that
shows the suggestions or the disambiguation choices, and no intent-access authorization flow.
