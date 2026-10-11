# Theme 03: Dynamic instructions and profiles

Branch `parity/theme-03`, built on `parity/integration`. Code was not built or run (the worktree rule is no builds).

## What was built

`springboard/ui/src/model_session.rs` (new, one `pub mod model_session;` line in `springboard/ui/src/lib.rs`). A pure
model of a model session with:

- `DynamicInstructions`: an ordered list of parts. A fixed part is always included. A dynamic part is a closure that runs
  before every request and returns `Option<String>`; `None` leaves the part out. The closure sees a `Frame` with the turn
  number and the session properties.
- `Profile`: a named set of dynamic instructions.
- `SessionProperties`, `PropertyKey`, `PropertyValue`: typed session properties (`Bool`, `Int`, `Text`). Each key has a
  default value, and a value of another kind is refused with `PropertyError::TypeMismatch`.
- `Session`: history (prompt and response entries), the current profile, and the property values. `switch_profile` keeps
  the history and changes the next request's instructions. `prepare(budget, reserve)` builds the transcript for the next
  request, with the current instructions first, and trims old turns with the existing `context_window::fit`.

It reuses `transcript::{Entry, Segment, Transcript}` and `context_window::fit`. It runs no model and makes no request.

## Apple API names mirrored

Official pages fetched with `tools/parity/docc.py` and rendered with REA:

- `LanguageModelSession.Profile`: "A profile that contains dynamic instructions." `init(_:)` creates it from dynamic
  instructions. Its page says a profile binds dynamic instructions to session-level configuration values.
- `DynamicInstructions`: "A type that represents dynamic instructions." Its body is evaluated before every request, so it
  can branch on current app state.
- `LanguageModelSession.SessionProperty` and `SessionPropertyKey`: a key has a `defaultValue` and a `Value` type.
  `SessionProperty` reads a value from `SessionPropertyValues` inside profiles and dynamic instructions.
- `LanguageModelSession.init(profile:history:)`: a session created with a profile and existing history. This matches
  `Session::with_history` and `Session::switch_profile`.
- `Instructions`, `Prompt` (entries of the `Session` history). `ContextOptions` is not mirrored; `prepare` only takes a
  token budget and reserve, from `context_window::fit`.

Not mirrored: `DynamicInstructions.ForEach`, `ConditionalDynamicInstructions` and the result builder (the closure
returns `Option` instead), `DynamicProfile` and its modifiers, `onToolOutput` lifecycle callbacks, `GenerationOptions`,
and `Tool` calling. There is no model runtime, so nothing reads the transcript this module builds.

## REA evidence

- `ev_fbfdd383c1a4566310bc7e7231a2d1b4f730429d8d051c3e7dd520a68f3b51e0`: `LanguageModelSession.Profile` page
  (https://developer.apple.com/documentation/foundationmodels/languagemodelsession/profile).
- `ev_65ae9981920ee47e1a523ed01deebd4ae53e0d0cde590577d96d93d1568adda0`: `DynamicInstructions` page
  (https://developer.apple.com/documentation/foundationmodels/dynamicinstructions).
- `ev_b3eea547db5896d907f25e5bb9e9a88f60dad2afc21930dbfaa484a604128508`: `LanguageModelSession.SessionProperty` page
  (https://developer.apple.com/documentation/foundationmodels/languagemodelsession/sessionproperty).

## Tests

`#[cfg(test)] mod tests` in `model_session.rs` has 7 behaviour tests:

1. Fixed and dynamic parts appear in order, and empty parts are left out.
2. Dynamic instructions are re-evaluated per request, and the turn number changes.
3. A part reads a session property, and changes when the property is set.
4. A property reads its default until set, and a value of another kind is refused without changing anything.
5. Switching the profile keeps the history and changes the next instructions.
6. `prepare` keeps the instructions and drops old turns over budget (checked against the `fit` arithmetic).
7. `prepare` adds no instructions entry when every part is empty.

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| foundation-models | Dynamic instructions and profiles | partial | springboard/ui/src/model_session.rs (Profile, DynamicInstructions, Session::instructions) |
| foundation-models | Session properties | partial | springboard/ui/src/model_session.rs (PropertyKey, PropertyValue, SessionProperties) |
| foundation-models | Sessions and prompts | partial | springboard/ui/src/model_session.rs (Session::prompt, respond, prepare). GenerationOptions is not modelled. |

## Notes for the lead

Wave-1 code already on this branch covers some rows of this theme, but their matrix status still says gap. I did not
claim these as closed, so please reconcile them:

- foundation-models "Context window and token budget": `springboard/ui/src/context_window.rs` (estimate, `fit`).
- foundation-models "Session transcripts": `springboard/ui/src/transcript.rs`.
- foundation-models "Guided generation (Generable, Guide, GenerationSchema)": `springboard/ui/src/generation_schema.rs`.
- core-transferable "Transferable protocol and TransferRepresentation builder", "Proxy representation and transfer
  visibility": `springboard/sdk/src/transfer.rs`.
- core-ai "Inference functions and values": `springboard/sdk/src/inference.rs`.
- foundation-models "Multimodal prompt attachments": `Segment::Attachment` in `transcript.rs` only, and no model input path.

`tools/parity/parity.py search` failed in this worktree with a corrupt semantic-embedding file (`numpy` BadZipFile on
`tools/parity/semantic.py` `Embedder`), so the duplicate check was done by reading neighbouring modules and grepping
for profile, instruction and property terms.
