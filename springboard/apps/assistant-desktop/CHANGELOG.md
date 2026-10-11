# Changelog

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
