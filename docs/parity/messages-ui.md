# Messages UI (slug: messages-ui)

Slice: the Messages app (`springboard/apps/messages-desktop`), the Matrix window, notifications and credentials.
Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Applications/Messages.app/Contents/Info.plist`,
but the bundle is not extracted under `ref27/` yet. No REA evidence was taken in this round. The UI below is worked from
the existing app. Recheck the bundle when it lands.

## What is in the app now

| Item | BifrOSt path | State |
| --- | --- | --- |
| Sidebar rooms with unread badges, newest-first order, preview line | `springboard/apps/messages-desktop/src/draw.rs` `draw_sidebar`; `src/model.rs` `apply_sync` | Done (wave 6). Pixel check needs a Midget run. |
| Timeline bubbles (own right, others left with sender name) | `src/draw.rs` `draw_timeline`, `timeline_rows` | Done (wave 6). |
| Day headings (Today, Yesterday, date) | `src/model.rs` `day_of`, `starts_day`, `day_label`; `src/draw.rs` `TimelineRow::day`, `day_height` | Done in this round. Days are UTC. |
| Typing indicator (received) | `src/protocol.rs` `typing`, `RoomUpdate::typing`; `src/model.rs` `Room::typing`, `typing_text`; `src/draw.rs` typing line | Done in this round. Sending our own typing state is not done. |
| Composer, Send button, Return sends | `src/window.rs` `key`, `send`; `src/draw.rs` `draw_composer` | Done (wave 6). |
| Image attachments (picker, size limit, MIME check, upload then send) | `src/window.rs` `pick_image`, `document`, `read_picture`; `src/sync.rs` `send_image` | Done (wave 6). Received pictures show their name only. |
| Notifications for other people's messages after first sync | `src/model.rs` `apply_room`; `src/window.rs` `Event::Synced` (`ctx.notify`) | Done (wave 6). |
| Dock badge with total unread | `src/window.rs` `badge` | Done (wave 6). |
| Credentials in the shared Passwords group (`is.bifrost.passwords`) | `src/accounts.rs`; `src/window.rs` `Pending::*` | Done (wave 6). The manifest declares `keychain_groups`. |

## Still open

- Typing: the app does not tell the server it is typing (`PUT /rooms/{id}/typing/{user}`). Needs a send job and a debounce.
- Received pictures are listed by name only. Fetching `mxc://` content and decoding it needs a decoder the app does not link.
- Day headings use UTC. The lab machines run on UTC+0. A local zone needs the SDK `locale` formatter, not wired in this round.
- Needs a Midget check: bubble alignment under a heading, the typing line at the bottom of the timeline, and the badge on a selected row.
