# Foundation Models: parity pass (pure Rust models)

Official reference: https://developer.apple.com/documentation/foundationmodels (macOS 26 and later; the
ContextOptions page is macOS 27).

This pass adds three pure, std-only models to `bifrost-ui` (`springboard/ui/src`). They have no
model runtime behind them, so they describe and check a conversation and its output. They do not
generate text. The Foundation Models row set moves from gap to partial on three rows.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/transcript.rs` | `Transcript`, `Transcript.Entry`, `Transcript.Instructions`, `.Prompt`, `.Response`, `.Reasoning`, `.Segment`, `.TextSegment`, `.StructuredSegment`, `.AttachmentSegment`, `.ToolCalls`, `.ToolCall`, `.ToolOutput`, `history`, `init(entries:)`, `structuredTranscript` | A linear history, oldest first. Only a leading instructions entry counts as the instructions, and `history` excludes it. `text()` joins prompt and response text. `structured()` returns tool calls, tool outputs and responses as typed lists in order. |
| `springboard/ui/src/context_window.rs` | `ContextOptions` (the token budget part, "Managing the context window") | `estimate_tokens` counts characters at four per token, rounded up. `fit(transcript, budget, reserve)` keeps the instructions and the newest history that fits. It starts the kept history at a prompt, so a tool output or response is never kept without its prompt. It reports what it dropped and whether the instructions alone are over budget. |
| `springboard/ui/src/generation_schema.rs` | `GenerationSchema`, `GenerationSchema.Property`, `Generable(description:)`, `Guide(description:)`, `GeneratedContent`, the string enumeration form `init(type:description:anyOf:)` | `Kind` describes a schema: text, number (optionally in a range), boolean, string choice, array and object. `Property` carries a name, a description and an optional flag. `validate(schema, value)` returns every issue with a JSON-style path such as `$.tags[1]`, and refuses unknown members and non-finite numbers. `describe()` writes a compact form for a prompt, as `includeSchemaInPrompt` does. |

`springboard/ui/src/lib.rs` has one `pub mod` line per new module. The matrix data is in
`docs/parity-matrix/data/foundation-models.json`.

Each module has its own `#[cfg(test)] mod tests` with behaviour tests: leading-instructions handling,
text and structured extraction, trimming order, prompt-aligned history, over-budget instructions, the
response reserve, schema paths, choice and range checks, unknown members, and the prompt description.

## Evidence

- REA passive observation of the rendered official landing page, 2026-10-11:
  `ev_b0de7481b8a0f27d59f8ce64b30d38ee78d3faa1f280313898c55745d9217451`
  (title "Foundation Models | Apple Developer Documentation"; the matrix's earlier capture of the same
  page is `ev_ab0c276e8f08660eff146ce88e3a628c67e0301757f4d384a0819607760d8663`).
- Official API shape from DocC (`tools/parity/docc.py`) for `foundationmodels/transcript`,
  `foundationmodels/contextoptions` and `foundationmodels/generationschema`.

## Features and status after this pass

| Feature | Status | BifrOSt |
|---|---|---|
| Session transcripts | partial | `transcript.rs` (the Assistant keeps its own store) |
| Context window and token budget | partial | `context_window.rs` (a character estimate, not a tokenizer) |
| Guided generation | partial | `generation_schema.rs` (validation only, nothing constrains output) |
| Tool calling | gap | `ToolCall` and `ToolOutput` entries exist; no model invokes a tool |
| On-device system language model, sessions, streaming, custom providers, dynamic profiles, session properties, safety, Private Cloud Compute | gap or n/a | unchanged |

## Remaining gaps

- No on-device language model and no SDK API for model sessions, instructions, prompts or generation
  options. An app cannot generate text without a network call.
- Guided generation is validation only. No model output is constrained to a schema, and there is no
  typed decoding into generated values.
- No model-invoked tool calling. App Intents (`springboard/sdk/src/intents.rs`) are not exposed to a
  model.
- The token budget is a character estimate. There is no model tokenizer and no cached-state reuse
  across turns.
- Transcripts are not shared across apps. The Assistant keeps its own store.
- Multimodal input to a model, dynamic instructions and profiles, custom language model providers,
  session properties and output safety filters are not started.

## Verification

Build and tests were not run in this pass, by request: another stage builds and tests the workspace.
The owner name rule was checked with `python3 tools/check-names.py` (exit 0).
