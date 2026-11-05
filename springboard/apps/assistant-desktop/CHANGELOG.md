# Changelog

## 1.4.8

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 1.4.7

- The empty-state text is hidden while the Settings sheet covers it.

## 1.4.6

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 1.4.5

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 1.4.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 1.4.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 1.4.2

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 1.4.1 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.4.0 — 2026-10-12

- The Assistant can use two tools while it answers: the current date and time (in UTC), and a word count. The model decides when to use them.
- Each tool the model uses shows in the reply it belongs to, above the reply's text: the tool's name, the arguments it chose and the result it got. The tool calls are kept with the conversation.
- When no model can be reached (you are offline, the service cannot be reached, or it is overloaded), a banner at the top of the chat says so. Your message is kept, so you can send it again later.
- Replies still appear as they are written, as before.

## 1.3.0 — 2026-10-12

- Each request is now built as a session of its conversation: the messages you sent and got back, under the instructions of the skill the request was routed to. A general request still goes out with no instructions, as before.
- A long conversation keeps its newest messages within the model's context budget. Older messages past that budget are not sent, and the rest of the chat stays on screen as before.
- Tool calls are not listed in replies yet: the Assistant does not offer any tools to the model.

## 1.2.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.2.0 — 2026-10-12

- Every answer is checked before you see it. An answer longer than 20,000 characters is cut, and the chat says so. Phrases you list in `guardrail.txt` in the app's data folder (one per line, lines starting with `#` are notes) are masked as `[removed]`. A part of a listed phrase is never shown while the answer is still arriving.
- Each request is routed to a skill by the words in it. A request about code, writing, a summary or a translation is sent with the instructions for that skill; a request with none of those words goes out as before.
- Chat > Diagnostics (Cmd+Shift+D) scores the newest answers of the open chat. Two rule checks always run (not empty, within the guardrail). With an API key, the model also rates each answer on helpfulness and clarity, from 1 to 5. The sheet shows the mean, median and range of each score and which skill the last request went to. Run Evaluation makes one request per answer and rating, so it costs a little; Escape closes the sheet.

## 1.1.0 — 2026-10-12

- When the machine has no network, a message is not sent: the composer keeps the text and the chat says "You are offline. Connect to a network to send this message." The empty chat says so too. The chat asks the desktop once for its connection state and then follows its changes.
- Chat > Copy Answer (Cmd+Shift+C) copies the newest answer to the desktop clipboard as formatted text and as plain text, so it pastes with its bold, code and code blocks into rich editors and as plain text everywhere else.

## 1.0.2

- Networking uses the SDK `URLSession` (`bifrost_app::url`) instead of the shell-internal HTTP client; its own chunked decoder is gone.
- Conversations are stored with `codable!` (`bifrost_app::coding`); an entry missing a title or date still loads.

## 1.0.1

- Compliance pass against the SDK, kit and design gate: spacing, radii and the settings sheet geometry come from the named tokens (`SPACE_*`, `prefs_layout::m`), colours from the scheme tokens, and no size depends on a fixed resolution.
- The app menu is titled Chat, so it no longer repeats the shell's File menu.
- Replies use a background dispatch queue and wake the loop through a socket (`wake_fd`) when text arrives, instead of a raw thread and polling.

## 1.0.0

- First release: a chat client for the Anthropic Messages API in the Golden Gate style. A source-list sidebar of conversations, a transcript with right-aligned user bubbles and plain assistant replies (paragraphs, bold, inline code, code blocks), and a composer (Return sends, Shift+Return adds a line, Stop while streaming).
- New Chat (Cmd+N) and Settings (Cmd+,) to paste the API key (kept in the Keychain) and pick the model (Claude Sonnet 5.5, Opus 5.5 or Haiku 5.5).
- Replies stream in as they arrive; API and network errors show as an inline row. Conversations are saved as JSON in the app's data directory.
- Follows Light and Dark.
