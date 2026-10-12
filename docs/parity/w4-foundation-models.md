# Wave 4: Foundation Models (tool calling, provider, attachments)

Status: pure models in `bifrost-ui`, host unit tests written, not built or run (wave rule: no builds in this round).
Not wired into the SDK, the Assistant app or the shell yet, so no row is `parity`.

## What was built

- `springboard/ui/src/model_tool.rs`: `Tool` (name, description, object parameter schema from `generation_schema::Kind`,
  handler) and `ToolSet` (registration checks, instruction text for a request, `call` and `output_for`). Arguments are
  parsed as JSON, checked against the schema, and only then is the handler run. A failed call is returned to the model
  as the text `error: ...` so the turn can go on.
- `springboard/ui/src/model_provider.rs`: `ModelProvider` trait (`name`, `accepts(media)` defaulting to false,
  `generate(&Transcript) -> Reply`), `Reply` (`Respond` or `Call`), and `run_turn`, which fits the session history to the
  budget, adds the tool descriptions to the instructions, runs tool rounds up to a limit and records each entry in the
  session. The transport lives in the caller's implementation; there is no network code.
- `springboard/ui/src/model_attachment.rs`: `Media` (PNG, JPEG, GIF, WebP, sniffed from the leading bytes),
  `Attachment::from_data` with a size limit, `labelled`, and `segment()` giving the existing `Segment::Attachment`.
- `springboard/ui/src/generation_schema.rs`: `parse(&str) -> Result<Value, ParseError>`, a JSON reader with a nesting
  limit, used to read tool arguments and any other model text.
- `springboard/ui/src/model_session.rs`: `Session::record(entry)`, so a turn can append tool calls and tool outputs.
- `springboard/ui/src/lib.rs`: three `pub mod` lines.

## Official API mirrored

| Apple (official page) | BifrOSt |
|---|---|
| `Tool`: `name`, `description`, `parameters`, `call(arguments:)`, `includesSchemaInInstructions` (developer.apple.com/documentation/foundationmodels/tool) | `Tool::new`, `name()`, `description()`, `ToolSet::call`, `without_schema_in_instructions` |
| `Tool.Output` used by the model in later turns | `ToolOutput` text recorded by `run_turn` |
| `LanguageModel` protocol: `capabilities`, `supportsDataAttachmentType(_:)` (developer.apple.com/documentation/foundationmodels/languagemodel) | `ModelProvider::name`, `ModelProvider::accepts` |
| `Attachment`: `init(_:)` from data, `label(_:)` (developer.apple.com/documentation/foundationmodels/attachment) | `Attachment::from_data`, `Attachment::labelled` |

REA evidence of the rendered official pages:

- Tool: `ev_2b8f490ca476248bedf74c747a195097e0ba0c4007fba02a2bd9dcb4bdf86c35`
- Attachment: `ev_14e8df9884673afe68c1f77ad0bb5177c48b0abb71a1053523ee5311fd2ae7d0`
- LanguageModel: `ev_b3db03bae3119a40a9516f9ebfc21c7262f052c7674ed89f2b98038ae34584da`

## Matrix rows closed (framework: Foundation Models)

| Framework | Feature (as in data/foundation-models.json) | New status | BifrOSt path |
|---|---|---|---|
| Foundation Models | Tool calling (Tool) | partial | springboard/ui/src/model_tool.rs |
| Foundation Models | Custom language model provider | partial | springboard/ui/src/model_provider.rs |
| Foundation Models | Multimodal prompt attachments | partial | springboard/ui/src/model_attachment.rs |

## Not closed, and why

- Tools are not `Sendable`, so they cannot run concurrently as the official page describes. `Tool` holds a `Box<dyn Fn>`.
- Attachments have no orientation argument. The official page lists one, and its values were not read from the docs here.
- `Attachment` does not check that the chosen provider accepts the media type; `ModelProvider::accepts` exists for that
  check but no caller uses it yet.
- No guardrail output filtering, no on-device model, no model-level streaming of partial output.
- The `Tool` argument schema is the existing `generation_schema` object type. Official `GenerationSchema` features such as
  dynamic schemas are not covered.
- Nothing is wired to the Assistant app; the Assistant still builds its Messages API requests by hand.
