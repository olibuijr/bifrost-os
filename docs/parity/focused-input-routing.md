# Focused input routing (round 75)

Slice: text input routing from the shell to app processes. Matrix family: InputMethodKit (`docs/parity-matrix/data/inputmethodkit.json`).

Evidence: the matrix row notes, the code on `wave-base` (`springboard/ui/src/compose_method.rs`, `springboard/ui/src/text.rs`, `springboard/sdk/src/view/forms.rs`, `springboard/src/shell/compose_window.rs`, `springboard/src/shell/apphost.rs`). No REA evidence was gathered for this slice and the reference index was not consulted. The behaviour is BifrOSt's own routing of its composition messages.

## What was already there (wave 5 base)

- The shared compose method (`compose_method::Composer`, `ComposeTarget`, `type_text_as`, `accent_hold`) and the Japanese kana flag inside each process (`set_japanese_source`).
- Every SDK text field types through `TextEditor::type_text`, so an app's own field already composes apostrophe lists, kana and its candidate list (drawn by `widgets::candidate_list`).
- The shell's Search field used the same compose method.

What was missing: the shell process set the kana flag only for itself, so app processes always read it as off; and a held letter only opened a list in Search.

## Rows

| Matrix feature | API (reference name, not in code) | BifrOSt path | Done (host, not built) | Still needs Midget |
|---|---|---|---|---|
| Text input client for apps (caret, text state, traits) | input source to the client | `proto/src/lib.rs` `ToApp::InputSource { japanese }` (tag 0x162); `sdk/src/runtime.rs` records it with `compose_method::set_japanese_source` on receipt (every SDK app, no per-app edit); `src/shell/apphost.rs` `AppHost::sync_input_source` sends on first contact and on change | yes | an app field reads the Romaji flag after a real switch (Ctrl+Space) |
| Key binding and text events to the input method | accent hold to the client | `proto/src/lib.rs` `ToApp::AccentHold { letter }` (tag 0x163); `sdk/src/view/forms.rs` `apply_text_input` and `ui/src/text.rs` `TextEditor::accent_hold` take back the letter and open its list; `src/shell/compose_window.rs` `accent_hold` sends it to the app in front when no Search field is open; `accent_popup_ready` accepts a focused app field (`apphost.rs` `ext_text_focused`) | yes (the editor's list is covered by a unit test; the shell send is not) | a held letter in a real app field opens its accents, and a digit commits the pick |
| Candidates window | candidate panel over the field | the app's own list (`widgets::candidate_list`), drawn by the SDK field; no second list in the shell | yes (shared with the existing field path) | list position in an app field |

## Behaviour

- Held letter in a focused app field on a Latin source: the app takes back the letter typed at the press and shows it as marked text over its accents. Digits pick. Any other key ends the list and keeps the letter.
- Search keeps the accent hold when it is open.
- Japanese source: no accent list, as before.

## Adopt and delete

- The accent hold for app fields goes through the existing compose method and candidate list, so no parallel shell candidate window was built. No superseded code was removed in this slice.
- No app change, so no version bump.

## Open

- The shell cannot see an app's compose state. A hold while the app has an apostrophe armed is ignored by the app (`accent_hold` returns false) and the letter just repeats. Acceptable, not fixed.
- With the flag set, an app field composes kana through `TextEditor::type_text`, the same path as Search. Needs a Midget check.
