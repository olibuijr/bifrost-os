# Wire 6: Assistant (streaming replies, tool calls, availability banner)

Branch `wire6/assistant`, Assistant 1.4.0 (from 1.3.0). Round two of the Foundation Models rows. Streaming text was
already wired in 1.0.x; this round adds the tool-call loop, shows the calls inline, and adds an availability banner.
The models are unchanged: the app adds `src/tools.rs` and a tool-use loop in `src/api.rs`, and uses the pure
`model_tool` set (`springboard/ui/src/model_tool.rs`) and the transcript records (`springboard/ui/src/transcript.rs`,
`ToolCall`, `ToolOutput`).

## What is now wired

| Model | How the app uses it |
|---|---|
| `bifrost_app::controls::model_tool::{Tool, ToolSet}` (`springboard/ui/src/model_tool.rs`) | `tools::tool_set()` holds `current_time` and `count_words`. `ToolSet::output_for` runs each call the model makes: the arguments are parsed and checked against the schema, then the handler runs. A failed call returns `error: ...` to the model, so the turn goes on. |
| `bifrost_app::controls::transcript::{ToolCall, ToolOutput}` (`springboard/ui/src/transcript.rs`) | The call the model made and the output handed back. |
| `bifrost_app::controls::generation_schema::{Kind, Property, Value}` | The tools' parameter schemas (checked by `ToolSet`). |

The request now declares `tools` (the Messages API tool definitions from `tools::tools_json`, which must match
`tools::tool_set`). Each streamed `tool_use` block is collected by `api::run_turn`. When the turn ends with calls, the
app runs them, sends the results as `tool_result` blocks and streams the next request, for up to `MAX_TOOL_ROUNDS` (4)
rounds. Text from every round goes into the same reply.

Availability: `api::is_unreachable` (`Cannot reach ...`, HTTP 5xx, overloaded) and the offline state drive a banner at
the top of the chat (`view.rs` `banner`, `draw_banner`). An empty chat keeps its own "Add your API key" or offline hint.

## How a user reaches it

- Tools: nothing to set. Ask for the time ("what time is it in UTC?") or a word count ("how many words are in: ...").
  Each call shows as a code block above the reply's text: `current_time({})` then `→ 2026-10-12T...Z`, or
  `count_words({"text":"..."})` then `→ 4 words`.
- Streaming: replies still appear as the text arrives.
- Banner: with the network off, or when the service cannot be reached or is overloaded, a card at the top of the open
  chat says "No model is reachable right now. Your message is kept; try again when the connection is back." Offline
  says "You are offline. Connect to a network to chat." The message you sent stays in the chat.
- Stored conversations keep their tool calls (`conversations.json`, a `tools` list per message). Conversations saved
  before this version load with none.

## How to verify on Midget

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open Assistant (`bifrost pc midget open-app is.olibuijr.assistant.desktop`) with an API key set.
3. Send "what time is it in UTC?". The reply should show a `current_time({})` block with a `→` line holding an ISO
   time that matches the Midget clock, and then the answer text.
4. Send "how many words are in: one two three four". The block should read `count_words({"text":...})` and `→ 4 words`.
5. Send "hi". No tool block should appear for a plain greeting.
6. Disconnect the network and send "hello": the composer keeps the text, the error line says "You are offline", and the
   banner appears at the top of the chat.
7. Screenshot with `bifrost pc midget screen /tmp/assistant-tools.png`; check Light and Dark (`bifrost pc midget
   appearance light|dark`). The main content area uses `CONTENT_BG`.

Host unit tests (not run this round; the lead builds once): `api.rs` tests `sse_text_deltas_errors_and_stop`,
`a_tool_block_streams_its_start_arguments_and_stop`, `a_turn_body_offers_the_tools`, `the_assistant_turn_keeps_its_text_and_calls`,
`an_empty_or_broken_input_is_sent_as_an_empty_object`, `an_unreachable_service_is_an_availability_failure`;
`tools.rs` tests `the_epoch_and_a_known_second_are_dated_correctly` (1 000 000 000 s is 2001-09-09T01:46:40Z),
`counting_words_reads_the_text_member`, `the_tool_set_runs_checked_calls`, `the_definitions_match_the_tool_names`;
`model.rs` tests `conversations_round_trip_through_json` (now with a tool record) and
`a_saved_message_without_tools_loads_with_none`.

## REA evidence

- `ev_eb9bc0764fd7f58fb6667c6b8dabb27468bd2c0665d30239f444ddf56bf022bd`: the rendered official page for `Tool`
  (tool calling: a name and a description go in the prompt, `call(arguments:)` returns an output that is handed back to
  the model, and back-to-back calls run when one tool's output feeds another).
- `ev_0df7bbc4355dd8f7cf5636bce351500f7c3a158fe07f42eb82e9295b40e71f85`: the rendered page for guided generation
  (`Generable`, a schema defined at runtime).
- `ev_c49ce402e8048a2a04ed25bba451a9a7678dfffb46c8d465af5965f7ddc9c187`: the rendered page for `SystemLanguageModel`
  (the on-device model, its availability; no on-device model exists in BifrOSt).
- `ev_ba2465e4ffe79a046b9dbd812c28b0c958ad17423c69ac4ba5f6eb739be85c77`: from wire 5, `LanguageModelSession` (kept
  unchanged).
- DocC topic lists: `python3 -I tools/parity/docc.py foundationmodels/tool --depth 1`.

## Remaining gaps

- Guided generation (`Generable`, `GenerationSchema`, `generation_schema.rs`): no skill asks for a structured reply yet, so
  no schema is sent and nothing constrains the model's output. The shape is ready for a skill that needs one (a
  `Kind` schema validated with `generation_schema::validate`, the reply parsed with `generation_schema::parse`).
- Tool calls show above the reply's text, not between the text segments of a reply that alternates text and calls.
- The Diagnostics judge (`safety::judge_call`) is a request too, so it is offered the tools. Its calls are ignored
  (`Event::Tool` is skipped) and only the reply text is read. Passing a no-tools flag to `api::stream` is a small follow-up.
- Only two tools exist (`current_time`, `count_words`). The app has no tool that reads its own data or other apps.
- The banner covers offline, unreachable and HTTP 5xx or overloaded states. Errors such as a rejected key still use the
  red error line only, and the banner does not appear for an empty chat.
- On-device model availability (`SystemLanguageModel`) is still a gap: the Assistant is a cloud client, so the banner
  reports network and service state, not a local model's state.
- Partial generation of typed values is not wired (the stream is text deltas and tool-input JSON only).
- Not verified on Midget yet. The matrix rows stay `partial` until the steps above pass.

## Matrix rows

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| foundation-models | Tool calling (Tool) | springboard/ui/src/model_tool.rs (model); springboard/apps/assistant-desktop/src/tools.rs (`tool_set`, `tools_json`), src/api.rs (`run`, `run_turn`, `Event::Tool`), src/view.rs (`Event::Tool` in `tick`, `tool_blocks`) |
| foundation-models | Streaming responses | springboard/apps/assistant-desktop/src/api.rs (`SseParser`, `Event::Delta`), src/view.rs (`tick`, `draw_transcript`); springboard/sdk/src/url.rs (`URLSession::bytes`) |
| foundation-models | Guided generation (Generable, Guide, GenerationSchema) | springboard/ui/src/generation_schema.rs (model; not wired, see Remaining gaps) |
| foundation-models | On-device system language model (SystemLanguageModel) | springboard/apps/assistant-desktop/src/view.rs (`banner`, `draw_banner`): a network and service availability banner, not a local model state (gap remains) |
| foundation-models | Safety guardrails for generated output | springboard/ui/src/guardrail.rs (unchanged from wire 2; tool results are not shown as reply text) |
