# Wave 4, theme 15: spoken input, spoken responses and tool calls

Theme: "Siri voice input and spoken responses" (`docs/parity-matrix/themes.json`, `theme-15`). The theme spans SiriKit,
Speech, Accessibility, Sound Analysis, Foundation Models, Authentication Services, Safari Services and SwiftData. This
round builds three pure models in `springboard/ui/src/` (`bifrost-ui`, std only, no hardware, no services). They close
three gap rows. They do not wire anything into the shell or apps, and nothing here has been built or run.

## What was built

| Module | What it does | Official API it mirrors |
|---|---|---|
| `springboard/ui/src/speech_assets.rs` | Inventory of speech models per module and locale: install states (supported, downloading with progress, installed, failed with reason), a plan of what still has to be installed, a worst-of status for a set of needs, and a capped set of locale reservations. Downloads nothing. Includes `normalize_locale` for tags such as `en_us` to `en-US`. | `AssetInventory` (`status(forModules:)`, `reserve(locale:)`, `release(reservedLocale:)`, `reservedLocales`, `maximumReservedLocales`, `assetInstallationRequest(supporting:)`), `AssetInventory.Status`, `SpeechModels` |
| `springboard/ui/src/voice_commands.rs` | Phrase grammar that turns a recognized utterance into a command (click, press a key, dictate, scroll, go back, show or hide numbers, names or grid). Numbers the actionable on-screen targets, and resolves a click to a target by name or number, reporting ambiguity. | `AccessibilityTechnology` (`voiceControl`, with the sibling technologies `switchControl`, `voiceOver` as context) |
| `springboard/ui/src/tool_calls.rs` | Tool trait (name, description, parameter schema, call), registry with unique names, instructions text listing each tool and its schema, arguments checked against the schema before the tool runs, a limit on calls per registry, and an ordered record of every call. Reuses `generation_schema` for schemas and validation. | `Tool` protocol (`name`, `description`, `parameters`, `includesSchemaInInstructions`, `call(arguments:)`, `Output`) |

Behaviour choices that are ours, not Apple's:

- `speech_assets`: `reserve` returns `Ok(false)` when every slot is taken instead of an error. The limit is passed in
  by the caller because the official page says only "a certain number" of reservations.
- `voice_commands`: the grammar is a small fixed phrase set. Key names are limited to a list in the module. Text after
  `dictate` keeps its capitals.
- `tool_calls`: `includes_schema` defaults to true here. The Apple page documents the property but this module does
  not claim its default. A registry limit of calls is our safety rule. Refused calls for the limit are not recorded.

## Official sources

- AssetInventory: https://developer.apple.com/documentation/speech/assetinventory (DocC: `python3 -I tools/parity/docc.py speech/assetinventory`)
- SpeechModels: https://developer.apple.com/documentation/speech/speechmodels
- Tool: https://developer.apple.com/documentation/foundationmodels/tool
- AccessibilityTechnology: https://developer.apple.com/documentation/accessibility/accessibilitytechnology
- ASAuthorizationController (read for context only, not built this round): https://developer.apple.com/documentation/authenticationservices/asauthorizationcontroller

## REA evidence (rendered official pages)

| Page | evidence_id |
|---|---|
| speech/assetinventory | `ev_8499106be5e3eeab4a52120bcf0da9a983629c194e8df7e7aff1435c84592edf` |
| foundationmodels/tool | `ev_67bf103efb4b79724a8d6513469dae5fe62efff5f001ffc25ccd2539ca0c1899` |
| accessibility/accessibilitytechnology | `ev_a0c7c067306347404a62d66fc7e72e5419344ba72e1cf25e9a0ae55eab8c5596` |
| authenticationservices/asauthorizationcontroller | `ev_c75bddbee13daec657f39be60895c9f768685d51cfe57bdf9599b8c9064b8449` |

## Matrix rows closed

Status `partial` in each case: the models are pure and unwired, and the rows do not reach `parity` until the feature
works on the VM and Midget.

| Framework slug | Feature (as in the data file) | New status | BifrOSt path |
|---|---|---|---|
| speech | Asset and model installation | partial | springboard/ui/src/speech_assets.rs |
| accessibility | Voice Control | partial | springboard/ui/src/voice_commands.rs |
| foundation-models | Tool calling (Tool) | partial | springboard/ui/src/tool_calls.rs |

## Not closed, and why

- Siri voice input, spoken responses and SiriKit intents: no speech recognizer or synthesizer is wired. The Assistant
  app is text only.
- Speech recognition on live audio: the recognizer and the audio pipeline are not connected; `speech_assets` only
  tracks the models a recognizer would need.
- Authorization requests (`ASAuthorizationController`): not built this round. Its REA evidence is recorded above for
  the next wave.
- Foundation Models on-device language model, guardrails and multimodal attachments: need a model runtime, which the
  repo does not have as a pure Rust component.

## Verification

- `python3 -I tools/check-names.py` exits 0 after the new files were added.
- No builds or tests were run, per the task rules. The `#[cfg(test)]` modules in each file hold the expected values
  worked out by hand from the rules above (for example, reservations with a limit of 2, the status priority order, the
  numbering of actionable targets, and the argument check that stops a tool from running).
