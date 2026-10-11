# Assistant

Chat client for the Anthropic Messages API (`POST /v1/messages`, `stream: true`).

- `src/api.rs`: the HTTPS request on a background thread (the Deildu TLS transport: rustls + graviola) and the SSE parser (`content_block_delta` / `text_delta`, `error`, `message_stop`).
- `src/model.rs`: conversations (JSON files in the data dir), settings, Markdown blocks.
- `src/view.rs`: sidebar, transcript, composer and the Settings sheet.
- The API key lives in the SDK keychain (service `is.olibuijr.assistant`, account `anthropic-api-key`); the model is in `settings.json`.
- Known limits: the composer edits at the end of the text (no caret movement or selection yet); code blocks use the system font because the shell ships no monospace face.
