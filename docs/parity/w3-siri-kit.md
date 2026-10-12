# Parity: SiriKit (wave 3)

Two gap rows of the SiriKit matrix now have a pure-Rust model. No new crates, no unsafe, no hardware, no
network. Both models are in `springboard/ui/src` (bifrost-ui) and are not wired into the shell or apps yet;
the shell's Search matcher and the Settings privacy page are the next consumers.

Official references:

- https://developer.apple.com/documentation/sirikit/registering-custom-vocabulary-with-sirikit
- https://developer.apple.com/documentation/sirikit/specifying-synonyms-for-your-app-name
- https://developer.apple.com/documentation/sirikit/intent-phrases
- https://developer.apple.com/documentation/sirikit/requesting-authorization-to-use-siri

REA evidence (rendered official pages):

- `ev_0cbdd335b605ecf441c53ed6922b16197206c119e9428281eaa9a754a228d1f2`: Specifying Synonyms for Your App Name
- `ev_d851a9b237f2f30809f2bad764d05da571693ce5053a655df0e4989f99ee8084`: Intent Phrases
- `ev_a0950cce42b899b998fe3c05a637754ca84a140f759e70c16428866f7815f37e`: Requesting Authorization to Use Siri

## What was built

1. **Custom vocabulary, synonyms and intent phrases** (`springboard/ui/src/intent_vocab.rs`, new module)
   - Mirrors the synonym list for an app name (`add_name`, one entry per name, at most 16 per app) and the
     sample phrases that run an intent (`add_phrase`, at most 512 in all). Repeats of a name or phrase are
     accepted and add nothing.
   - `normalize` folds case and turns every run of non-alphanumeric characters into one space. `lookup`
     matches the normalized request exactly against names first, then phrases, and returns `Match`, `NoMatch`
     or `Ambiguous` (more than one different target, so the caller asks the user which one is meant).
   - `remove_app` drops an app's names and phrases when the app is uninstalled.
   - Limits: 128 bytes per normalized name or phrase. Wildcards, pronunciation hints and parameter vocabularies
     are not modelled; the lookup does not match a phrase inside a longer request.
2. **Authorization to use the assistant** (`springboard/ui/src/intent_access.rs`, new module)
   - `Consent` keeps the user's answer per app and `Service` (`Assistant`, `Directions`, the two services the
     official page names). `request(app, service, usage, allow)` records the first answer and returns the
     status; a later request does not change a decided answer, which matches the once-only prompt.
   - `usage` (the prompt text, the role of the usage-description key) must not be blank; the app id must not
     be empty.
   - `may_run` is true only for `Authorized`. `revoke` returns an answer to `NotDetermined`, `forget_app` drops
     both services of an app, and `authorized(service)` lists the allowed apps in name order for a Settings page.
   - The model shows no prompt; the caller shows it and passes the answer back.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/siri-kit.json`) | New status | BifrOSt path |
|---|---|---|---|
| SiriKit | Custom vocabulary, synonyms and intent phrases | partial | `springboard/ui/src/intent_vocab.rs` |
| SiriKit | Authorization to use Siri | partial | `springboard/ui/src/intent_access.rs` |

## Gaps that remain

- Spoken input and spoken dialog: nothing recognizes speech or speaks `IntentResult.dialog`.
- Nothing calls `lookup` yet: Search still matches titles and app names only.
- `intent_access` is not enforced by the control socket (`intent.<id>` runs without asking); the shell must
  call `Consent::may_run` before it dispatches a request.
- The Shortcuts app still does not list or run intents.
- Media and messaging intent domains (the rows for them stay gap).
