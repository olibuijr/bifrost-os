# Connect: Assistant to the system services (`is.olibuijr.assistant.desktop`)

Round: connect/assistant. The app is `springboard/apps/assistant-desktop/` (version 1.0.2, not bumped this round).
New glue: `springboard/apps/assistant-desktop/src/connect.rs`. Wiring: `src/view.rs` (`Assistant`) and `src/main.rs`
(the app hooks `service_reply`, `service_event`, `menu_action`, `main_menu`).

No `wire-app-assistant` note existed from the previous round, so nothing was waiting on a service. The features below
are new and were not in the matrix as shipped.

## What is connected

### 1. Network path (`network` service): offline state before sending

- **SDK client:** `bifrost_app::network_path` (`request_path`, `decode_reply`, `path_event`).
- **When it is asked:** once, when the chat is created at app start (`Link::new` in `Assistant::new`). After that the
  shell pushes `pathUpdate` events when the path changes; nothing polls.
- **What it changes:**
  - A send while the path is not `satisfied` (`unsatisfied` or `requiresConnection`) is held back. The composer keeps
    the text and the chat shows the error row "You are offline. Connect to a network to send this message."
    No conversation is created and no request is started.
  - In an empty conversation, the hint "You are offline. Connect to a network to chat." replaces the API-key hint.
- **Missing service:** until the shell answers, the link is unknown and sending is allowed. A reply of
  `no service network` leaves it unknown, so an older shell never blocks a send.
- **Code:** `connect::Link` (`offline`, `reply`, `event`); `Assistant::send` checks `link.offline()` before any state
  change; `Assistant::service_reply` / `service_event` route the replies and events.

### 2. Pasteboard (`pasteboard` service): copy an answer as rich and plain text

- **SDK client:** `bifrost_app::pasteboard_types` (`Board::general().write`, `Item::with`, the `HTML` and `STRING` type ids).
- **Where the user reaches it:** Chat menu, "Copy Answer" (Cmd+Shift+C). It is enabled when the open conversation has a
  finished answer and no reply is streaming.
- **What is written:** one item on the general board with two representations: `public.html` (paragraphs with bold and
  code spans, code blocks in `<pre><code>`) and `public.utf8-plain-text` (the same text with bullets as "• ").
- **Code:** `connect::copy_answer`, `connect::answer_html`, `connect::answer_plain`; `Assistant::copy_answer` (one
  service call per user action); the reply is taken in `Assistant::service_reply`, and an error is logged with `ctx.log`.

## How a user sees it

- Offline: type a message with the network unplugged and press Return. The message stays in the composer and the
  red row says why. In a new chat the empty-state text says "You are offline."
- Copy: after an answer finishes, open the Chat menu and choose Copy Answer, or press Cmd+Shift+C. Paste into a rich
  editor to keep the bold, code and code blocks; paste into a plain field to get the text.

## How to verify on the test PC (Midget)

Midget only (the lab VM is not used for this round).

1. Build and publish as usual, then `bifrost pc midget ota` until `phase=accepted`. Open the Assistant.
2. Offline: pull the Ethernet cable (Midget uses wired VLAN 20; Wi-Fi is the fallback, so disable both for a real
   offline check). Type "test" and press Return. Expect the error row "You are offline. Connect to a network to send
   this message." and "test" still in the composer. Take `bifrost pc midget screen` as evidence.
3. Online: plug the cable back in. Expect the offline row to stay until the next send (the error is only written on a
   send), and a new send to start streaming (with a key set in Settings).
4. Empty chat offline: with no messages, expect "You are offline. Connect to a network to chat." under "How can I help?".
5. Copy: send a question that returns bold text and a code block. When the reply has finished, open Chat > Copy
   Answer. In a rich text field (for example the Notes app), paste: bold and code survive. In a plain field, paste:
   the text with no markup.
6. Missing service: on a shell without `network` (an older build), the chat must still send. Only the offline check
   is skipped; the reply is "no service network" and the link stays unknown.

Host tests (not run in this round, per the no-build rule for this worktree): `cargo test -p bifrost-assistant connect`
covers the link state machine, the service calls (network path request, pasteboard write), and the plain and HTML
forms of an answer.

## Gaps

- **No per-message copy.** The copy action takes the newest answer only; a copy button on each bubble needs hit testing
  in the transcript and is a separate change.
- **Offline during a stream is not handled.** A reply already streaming when the path drops is left to fail through the
  HTTP client's error row; the app does not cancel it on a `pathUpdate`.
- **Reachability is not checked.** The network path says whether the machine has a route, not whether the API host
  answers (the `network` service has no probe). A route with no internet still sends and fails with the error row.
- **Copy is a flat general board.** The general board keeps items flat in the shell (see `wire-svc-pasteboard.md`). The
  two representations are written as two flat entries; the shell's clipboard takes the plain text, and Wayland
  clients are offered the HTML. The copy does not re-broadcast to the synchronous `NativePasteboard` mirror of other
  apps until their next copy.
- **Paste still reads the synchronous clipboard.** The composer's paste (`clipboard::text`) was not moved to the
  pasteboard service; rich paste into the composer is not handled.
- **No access prompt.** The pasteboard service has no access behaviour, so any app can read what the Assistant copies.
- **Matrix not updated.** This round does not edit `docs/parity-matrix/`; the rows for the network path and the
  pasteboard should gain the Assistant as a consumer when the round is merged.
