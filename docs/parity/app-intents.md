# Parity: App Intents (wave 2)

Three gap rows of the App Intents matrix now have a pure-Rust counterpart. No new crates, no unsafe, no
network or hardware access. The shell does not yet act on the new flags; they are declared, validated and
served over the control socket, and the shell's suggestion list is the next consumer.

Official reference: https://developer.apple.com/documentation/appintents

## What was built

1. **Execution modes and authentication** (`springboard/sdk/src/intents.rs`)
   - `IntentMode` (`Background`, `Foreground`) mirrors `IntentModes`. `Intent::modes` defaults to
     `[Background]`; `Intent::foreground()` adds the foreground mode once. Listed in `intent.list` as `modes`.
   - `Authentication` mirrors `IntentAuthenticationPolicy` (`AlwaysAllowed` default, `RequiresAuthentication`,
     `RequiresLocalDeviceAuthentication`). `Intent::authentication(..)` sets it; listed as `authentication`.
   - `Session { locked, authenticated }` and `Intent::may_run(session)` evaluate the policy. The RPC dispatch
     does not enforce it yet, because the control socket carries no device or sign-in state. The shell or app
     must call `may_run` before performing an intent that is not `AlwaysAllowed`.
2. **Follow-on and opens-app results** (`intents.rs`)
   - `IntentResult::continue_in_foreground()` sets `continueInForeground: true` in the reply.
   - `IntentResult::opens(intent_id, params)` sets `opens: {intent, parameters}` (`result(opensIntent:)`).
   - `Intent::check_result(&result, &declared)` runs on every perform, from `dispatch` and `run`:
     continuing in the foreground needs the foreground mode (a BifrOSt rule: the shell needs a window to bring
     forward); an `opens` target must be a declared intent and its parameters must validate.
   - Results without these fields keep the existing `{"value", "dialog"}` shape.
3. **Donations and discovery** (`springboard/sdk/src/donations.rs`, new module)
   - `DonationLog` mirrors `IntentDonationManager` (`donate`, `delete_matching` for `deleteDonations(matching:)`)
     and `RelevantIntentManager` (`relevant` for `updateRelevantIntents`, returning `Relevant` entries that play
     the role of `RelevantIntent`).
   - Ranking: each (intent, entity) pair scores the sum of `0.5^(age / 7 days)` over its donations. The log is
     capped at 500 donations and round-trips through `to_json` / `from_json`, so an app keeps it in its data
     directory. Persisting it and feeding the shell's Search ranking are not done yet.
   - Not covered: `AppEntityContext`, `donate(intent:result:)` results, and the Apple-side relevance inputs.

## Official API names mirrored

`AppIntent` (authenticationPolicy, supportedModes), `IntentModes`, `IntentAuthenticationPolicy`,
`IntentResult` (continueInForeground, opensIntent), `IntentDonationManager` (donate, deleteDonations(matching:)),
`RelevantIntentManager` (updateRelevantIntents), `RelevantIntent`.

## REA evidence (rendered official pages)

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/appintents | ev_c441b370fcc37cac5c16394d85f699df60d53a7ab076421d2ed3b4044f81eeb0 |
| https://developer.apple.com/documentation/appintents/intentdonationmanager | ev_07e4898c301b5579ed9104d127c501c09e2edc3ee586e95d276ad8c3cc3ce19d |
| https://developer.apple.com/documentation/appintents/intentmodes | ev_35fb14c9654979f869391090a4893cfab8587ecaa09fae7a6ac62f69f3f910d3 |
| https://developer.apple.com/documentation/appintents/intentauthenticationpolicy | ev_fe7d58a6933bea97878716c24c975df6713e081cbe2fc99e4f807fb6512a6061 |

The DocC facts used (`tools/parity/docc.py appintents/...`): IntentModes has `background` and `foreground`
values; IntentAuthenticationPolicy has `alwaysAllowed`, `requiresAuthentication`,
`requiresLocalDeviceAuthentication`; IntentDonationManager has `donate(intent:)`, `donate(intent:result:)` and
`deleteDonations(matching:)`; RelevantIntentManager has `updateRelevantIntents(_:)`.
The default authentication policy and the donation half-life are BifrOSt choices, not Apple facts.

## Matrix rows closed

| Framework slug | Feature (as in docs/parity-matrix/data/app-intents.json) | New status | BifrOSt path |
|---|---|---|---|
| app-intents | Execution modes and authentication | partial | springboard/sdk/src/intents.rs |
| app-intents | Follow-on and opens-app results | partial | springboard/sdk/src/intents.rs |
| app-intents | Donations and discovery | partial | springboard/sdk/src/donations.rs |

Note for the lead: the matrix row "App enums" (status gap) is stale. `ParamType::Enum` with
`enum(a|b)` parsing and validation already exists in `springboard/sdk/src/intents.rs` and has tests. It was not
changed here because the matrix is updated centrally.

## Still open in this framework

Snippets and result views, common data types (Files, Currencies, People, Placemarks, Measurements), Doubles as
an intent parameter, typed intent errors (needs-value, confirmation), Focus filters, widgets and controls, schema
domains. Enforcement of the authentication policy at dispatch, and the shell's use of donations and the
foreground flag, are the next steps.
