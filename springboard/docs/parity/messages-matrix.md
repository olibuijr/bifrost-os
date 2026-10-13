# Messages: Matrix client (slug messages-matrix)

Messages is the daily chat app: a Matrix client over the homeserver's client-server API (`/_matrix/client/v3`). It is
not an Apple framework row; the matrix in this slice is the app's own feature list, below.

## Where it lives

| Part | Path |
|---|---|
| Wire format (URLs, bodies, answers; pure, unit tested on parsed JSON) | `springboard/apps/messages-desktop/src/protocol.rs` |
| HTTPS calls (login, sync, send text, upload and send image, read receipt) | `springboard/apps/messages-desktop/src/api.rs` |
| Saved logins in the shared passwords group | `springboard/apps/messages-desktop/src/accounts.rs` |
| Rooms, timelines, read markers, notifications, sign-in form (pure) | `springboard/apps/messages-desktop/src/model.rs` |
| Background jobs (sign-in, sync loop, sends) on threads with a wake pipe | `springboard/apps/messages-desktop/src/sync.rs` |
| Window flow, input, Passwords and picker replies | `springboard/apps/messages-desktop/src/window.rs` |
| Layout, drawing and hit testing | `springboard/apps/messages-desktop/src/draw.rs` |
| Installed app entry (SDK `App`) | `springboard/apps/messages-desktop/src/main.rs` |
| Shell HTTP client, one added method | `springboard/src/deildu/http.rs` (`request_with_headers`) |

The app is `is.olibuijr.messages.desktop`, crate `bifrost-messages`, version 0.1.0, full-size window, permissions
storage, network, keychain, notifications and documents, keychain group `is.bifrost.passwords`.

## Feature list and what is done

- **Sign in (homeserver, user, password).** Done in code. The sheet takes a host name (https is added) and refuses plain
  http, because a sign-in sends the password. Login is `m.login.password` with an `m.id.user` identifier. The answer gives
  the user id, access token and device id. `M_FORBIDDEN` reads as a wrong user name or password.
  Saved: after a typed sign-in succeeds, the password is saved as a login (service `matrix:<homeserver>`, account = user
  name typed, value = password) through `bifrost_app::passwords::save`, or `update` when the item exists. Next start:
  `passwords::list`, the first Matrix item, `passwords::fill`, then sign in. The token is not stored, so each start makes a
  new device.
- **Rooms list.** Done. Rooms come from the joined rooms in each sync. The name comes from the room's `m.room.name`
  (timeline first, then state), else its canonical alias, else its id. Rooms are ordered by their newest message. The
  row shows the name, a preview of the newest message and the unread count (badge).
- **Timeline.** Done. Text, notices and emotes are shown as text; a picture shows as "Sent an image: <name>". Encrypted
  messages show as "Sent an encrypted message" and redacted ones as "Message removed". Other kinds show as "Sent a file" and
  so on. Other people's messages are left, the user's own are right, in the tint colour. The timeline keeps the newest 200
  messages per room and sticks to the bottom until the user scrolls by hand.
- **Send text.** Done. The composer sends with Return or the Send button; `PUT .../send/m.room.message/{txn}` with a
  transaction id made from the time and a counter.
- **Send images.** Done. The open panel offers PNG, JPEG, GIF and WebP (the extension decides the type). The picture is
  read up to 25 MB, uploaded to `/_matrix/media/v3/upload`, then sent as an `m.image` message with its `mxc://` URI and
  mimetype and size.
- **Read receipts.** Done. Opening a room marks its newest message read (`POST .../receipt/m.read/{event}`), and new
  messages in the open room are marked read as the sync brings them. A receipt from this user's other device moves the
  local read position. The read position is kept per room so each event is marked once.
- **Notifications.** Done. After the first sync (history is not notified), each message from another user in a room
  that is not on screen calls `ctx.notify` with the room name and "sender: text". The dock badge shows the total unread
  count (`ctx.set_badge`, empty when nothing is unread).
- **Sync loop.** Done. One long-poll per account (`timeout=20000`, the shell's socket timeout is 25 s), its `next_batch`
  kept on the job thread. A failed call is reported and retried after 5 s. An unknown or forbidden token stops the loop.

## Still open (these need work, not a Midget check)

- End-to-end encryption (Olm and Megolm). Encrypted rooms show only the placeholder. This is the largest gap.
- Received pictures are shown by name only; the media download (`/_matrix/media/v3/download`) is not wired.
- Invitations: `rooms.invite` in the sync is ignored, so a pending invitation cannot be accepted yet.
- Only the first saved Matrix login is used. A second account is saved in Passwords but not signed in.
- A failed send drops the typed text (the composer is cleared before the send).
- No menu items; the composer, sheet and list are the whole UI.
- Account sign-out and the device list are not implemented.

## Evidence used

- Reference Messages.app resources: not extracted yet. `~/Projects/macos-ref/extract/ref27-index.tsv` lists
  `System/Applications/Messages.app` with its extensions, but `~/Projects/macos-ref/extract/ref27/System/Applications/`
  has no `Messages.app` directory. Layout here follows the app's existing siblings (Mail's three panes and sheets) and
  the HIG rules the shell uses. Re-check once the lead extracts Messages.app: `inspect_plist` on its Info.plist and
  `decode_interface_builder` on its nibs, then compare the sidebar, the bubble and the composer.
- No REA evidence ids yet. Nothing here was measured from the reference binaries.
- The Matrix client-server API (the v3 paths and the bodies used above) is public and was implemented from its
  documented shapes. The protocol tests feed the parser the same shapes a homeserver sends.

## Checks

Nothing was built or run here, as the brief asks; the lead builds and runs the tests. The unit tests are in each module
(`protocol`, `api`, `accounts`, `model`, `draw`, `window`). The shell change is covered by the existing HTTP client tests,
and `request_with_headers` is a thin pass-through of `send`.

## Shared-file edits

- `springboard/src/deildu/http.rs`: one new `pub fn request_with_headers` on `Client`, next to `post_with_headers`. Any
  method with caller headers and an optional body. Needed for the bearer-token GET and PUT; the old helpers are POST-only
  or GET without headers.
- `springboard/Cargo.toml`: `apps/messages-desktop` added to the workspace members.
- `springboard/Cargo.lock`: the `bifrost-messages` entry added (deps `bifrost-app`, `bifrost-core`, `springboard`).

## Matrix entries closed

None. The Apple parity matrix has no Matrix or Messages row for this slice.
