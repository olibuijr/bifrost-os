# Wire 5: Assistant (model sessions, transcript, context budget)

Branch `wire5/assistant`, Assistant 1.3.0. The app now builds each request as a session of its conversation, using the
pure `model_session` (profile and instructions) and `context_window` (budget) models from `springboard/ui/src/`. The
app adds one module, `src/session.rs`, and changes the send path in `src/view.rs` (`Assistant::send`). The models
themselves are unchanged.

## What is now wired

| Model | How the app uses it |
|---|---|
| `bifrost_app::controls::model_session::{Profile, DynamicInstructions, Session}` (`springboard/ui/src/model_session.rs`) | Each send builds a `Session` of the conversation (`session::session`). Its profile is named after the routed skill, or General, and its fixed instructions are the skill's system prompt (`safety::skill_prompt`). A general request has no instructions, so it still goes out with no system prompt. |
| `Session::prepare` and `context_window::fit` (`springboard/ui/src/context_window.rs`) | The request keeps the newest messages that fit `BUDGET_TOKENS` (150,000 estimated tokens) after the instructions, with `RESERVE_TOKENS` (the reply limit, 4,096) left free. The kept history starts at a prompt. |
| `bifrost_app::controls::transcript::{Entry, Segment}` (`springboard/ui/src/transcript.rs`) | The conversation's saved messages become prompt and response entries. Empty messages (a reply not yet started) are left out, as before. |

The request body is unchanged in shape (`api::request_body`): `model`, `max_tokens`, optional `system`, `messages`,
`stream: true`.

## How a user reaches it

- Chat: send a message as before. Requests with a skill word (code, writing, summary, translation) are sent with that
  skill's instructions, as in 1.2.0. Nothing to set.
- Long chats: the newest messages are sent. Older messages beyond about 150,000 estimated tokens are not sent. The chat
  still shows every message; nothing says that older ones were left out (see Remaining gaps).
- Availability: unchanged from 1.2.0. With no network, sending keeps the text and says "You are offline". With no API
  key, the empty chat says "Add your API key in Settings to start." If the service cannot be reached or returns an
  error, the error line under the transcript says so (`api.rs` `Cannot reach` and `error_message`).

## How to verify on Midget

1. Build and publish as usual (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).
2. Open Assistant (`bifrost pc midget open-app is.olibuijr.assistant.desktop`), add an API key in Settings.
3. Send "hi", then "write a short poem" and "what is my name" (a follow-up that needs the first message). The second
   answer should use the Writing instructions, and the third answer should refer to what came before, showing the history
   is sent.
4. Disconnect the network and send: the composer keeps the text and the chat says "You are offline".
5. Screenshot with `bifrost pc midget screen /tmp/assistant.png`; Light and Dark via `bifrost pc midget appearance`.
   The main content area uses `CONTENT_BG` (already the case in 1.0.1 and later).

Unit tests (host, not run in this round; the lead builds once): `src/session.rs` tests
`a_skill_request_carries_its_instructions_and_the_history_in_order`, `a_general_request_has_no_system_prompt`,
`empty_messages_are_left_out_of_the_session` and `the_oldest_messages_go_first_when_the_budget_is_small` (a budget of
250 tokens over three 100-token messages keeps only the newest prompt and reports two dropped).

## REA evidence

- `ev_ba2465e4ffe79a046b9dbd812c28b0c958ad17423c69ac4ba5f6eb739be85c77`: the rendered official page for
  `LanguageModelSession` (sessions keep state across requests, instructions set the model's role, the transcript
  records prompts and responses, and an oversized session throws a context-size error). Captured with
  `tools/parity/rea-page.sh` this round. The DocC topic list came from `tools/parity/docc.py` (foundationmodels/languagemodelsession).

## Remaining gaps

- Tool calls are not wired. The Messages request declares no tools, so the model cannot ask for one, and the stream
  parser (`api::SseParser`) ignores `tool_use` blocks. Listing tool calls in a reply needs the tool registry
  (`ui/src/tool_calls.rs`, `ui/src/model_tool.rs`) and a tool-use loop (`ui/src/model_provider.rs` `run_turn`), plus
  an app tool set. Not done here.
- Guided generation (`Generable`, `GenerationSchema`): no schema is sent with a request, so output is not constrained.
- Streamed partial generation: the reply streams text deltas only (as in 1.0.x); no typed partial objects.
- The session is rebuilt from the saved messages on every send. The app does not keep a live `Session` object
  between sends, and it does not use `Session::with_history` for restored chats beyond this rebuild.
- Trimming is silent: messages past the budget are not sent and the UI does not say so. A visible note is a small
  follow-up.
- Dynamic instructions (`DynamicInstructions::dynamic`) and session properties (`PropertyKey`) are not used; the profile
  has only fixed instructions.
- On-device model availability (`SystemLanguageModel`) is a gap: the Assistant is a cloud client, so there is no
  availability check of a local model. The states above are network and key states, not model states.
- The context budget counts characters divided by four. No tokenizer runs in the app, so the estimate can be off for
  code and for non-English text.
- Not verified on Midget yet (see above). The matrix rows stay `partial` until that test passes.

## Matrix rows

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| foundation-models | Sessions and prompts | springboard/ui/src/model_session.rs (model); springboard/apps/assistant-desktop/src/session.rs (wired, `session`, `request`), src/view.rs (`send`) |
| foundation-models | Session transcripts | springboard/ui/src/transcript.rs (model); springboard/apps/assistant-desktop/src/session.rs (`session`) |
| foundation-models | Context window and token budget | springboard/ui/src/context_window.rs (model); springboard/apps/assistant-desktop/src/session.rs (`request_within`, `BUDGET_TOKENS`) |
| foundation-models | Dynamic instructions and profiles | springboard/ui/src/model_session.rs (model); springboard/apps/assistant-desktop/src/session.rs (`profile`, fixed instructions only) |
| foundation-models | Tool calling (Tool) | springboard/ui/src/tool_calls.rs (model; not wired, see Remaining gaps) |
