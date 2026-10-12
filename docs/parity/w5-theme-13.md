# Wave 5, theme 13: App extensions (messaging slice)

## What was built

`springboard/sdk/src/messaging.rs` (pure Rust, std only, host unit tests) is one shared model for the
conversation side of two frameworks:

- `Person` (participant, `INPerson`): name, handle, current-user flag, `display_name`, `initials`, `same_as`.
- `Conversation`: participants, `others`, `is_group`, `display_title` (lists up to three names, counts the rest).
- `Notice`: an incoming message. `title` and `body` follow the group rule; `request()` returns a
  `usernotif::Request` on the conversation id as its thread, so the shell stacks one conversation together.
- `SendRequest`: `check()` (no recipients, more than `MAX_RECIPIENTS` = 50, current user as recipient, blank text)
  and `unique_recipients()`.
- `SearchRequest` and `search()`: text (case-insensitive substring), sender and conversation filters, newest
  first, limit (`DEFAULT_LIMIT` 25, capped at `MAX_LIMIT` 100).

`pub mod messaging;` was added to `springboard/sdk/src/lib.rs`. Nothing is wired into the shell or into an app yet.

## Official API names mirrored

- User Notifications: "Implementing communication notifications" (`UNNotificationContentProviding` and
  `UNNotificationAttributedMessageContext` are not mirrored). https://developer.apple.com/documentation/usernotifications/implementing-communication-notifications
- SiriKit Messaging: `INSendMessageIntent`, `INSearchForMessagesIntent`, `INPerson`.
  https://developer.apple.com/documentation/sirikit/messaging

## REA evidence

- `ev_06bb057d89a3288d81c549b0151ff214fd9ad51b20b304afd7373efbe73833c8`: rendered page "Implementing communication notifications".
- `ev_31bf3d417338cb25a5a48f3e0a42b310fb8ddc9dffab20eb43dc323a762b7f39`: rendered page "Messaging".

## Matrix rows closed (new status: partial)

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| user-notifications | Communication notifications | partial | springboard/sdk/src/messaging.rs (Notice, Conversation, Notice::request) |
| siri-kit | Messaging intents (send, search messages, contacts) | partial | springboard/sdk/src/messaging.rs (SendRequest, SearchRequest, search) |

## Not closed in this theme

Left as they are: App Intents widgets, controls and Live Activities; schema domains; SiriKit voice input; Apple Events;
StoreKit promoted purchases (needs a store-side promotion model); Swift actors (no Rust counterpart). Communication
notifications still lack a shell presentation (sender avatars, breakthrough) and contact lookup. Messaging still has
no contacts, attachments, edits or reactions.

## Verification

- `python3 tools/check-names.py`: exit 0 (no hits).
- No builds or tests were run, as the wave rules require. The unit tests in `messaging.rs` are hand-computed.
