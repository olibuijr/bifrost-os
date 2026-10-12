# Changelog
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
