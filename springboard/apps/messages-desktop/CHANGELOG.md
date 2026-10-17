# Changelog

## 0.6.0 — 2026-10-17

- End-to-end encryption on vodozemac (Olm and Megolm), in src/crypto.rs. Each sign-in makes a device with an Olm account; its device keys and signed one-time keys are uploaded (`/keys/upload`), the members' devices are queried (`/keys/query`) and checked against their self-signatures, and one-time keys are claimed (`/keys/claim`) for the devices without a session.
- Room keys travel as Olm `m.room.encrypted` to-device messages (`m.room_key`) and are taken in from the sync. Encrypted rooms (`m.room.encryption`) are sent as `m.room.encrypted` Megolm events, and incoming ones are decrypted before the window sees them.
- The Megolm session of a room rotates after 100 messages, after a week, or when the membership changes.
- A lock is drawn in the header of an encrypted room. A message this device cannot open shows "Unable to decrypt message", never the ciphertext.
- Pictures, files and reactions are refused in encrypted rooms for now, rather than sent in the clear.
- Not yet: device verification (every device is unverified, and nothing is shown as verified), key backup, cross-signing, the keychain store of the pickle (the account is kept in memory for the run), pickled Megolm sessions, encrypted pictures and reactions.
- Headless tests in src/crypto.rs and src/protocol.rs cover the key request and answer shapes, the to-device payloads, the rotation policy, the decrypt placeholder and a full room-key round trip.

## 0.5.4 — 2026-10-14

- Replaced the hand-drawn conversation list with the shared Golden Gate source-list widget and its matching row hit-testing.

## 0.5.3

- Made the keychain-backed account-row type explicit at its import so the declared keychain permission is traceable to its use.

## 0.5.2

- Exported the app's existing vector icon as the required 180 × 180 PNG, moved background jobs to utility dispatch queues with wake-pipe completions, and tied control radii to shared layout tokens.

## 0.5.1 — 2026-10-14

- Find and Emoji & Symbols are in the Message menu so the app does not duplicate the shell Edit menu.

## 0.5.0 — 2026-10-14

- Files dropped on the window go to the open conversation, as Attach File sends them: a picture as a picture, any other file as a file message. A drop is taken only when the account is signed in and a conversation is open.
- Tapping the quoted line of a reply outlines the message it jumps to, until the next press or the next conversation.
- Headless tests in tests/parity_app-messages.rs cover the reply outline, the drop policy, search, pins, the mute and unread switches, leaving a conversation, sign-out, and the reply and edit starts.
- Not yet: end-to-end encryption, a user directory search in New Message, server-side message search, skin tones, avatars in the details panel.

## 0.4.0 — 2026-10-14

- Emoji picker: a strip of groups under the search field (All, Faces, Hands, Nature, Food, Things, Signs). A group shows its own emoji, with its recently used ones first; a search still finds emoji in every group.
- Picker tests run headless in tests/parity_app-messages.rs.
- Not yet: end-to-end encryption, dragging files in, a user directory search in New Message, server-side message search, skin tones.

## 0.3.0 — 2026-10-13

- New Message takes several Matrix user ids, separated by commas, and starts a private group conversation; one id still starts a direct conversation.
- Pinned conversations keep the order they were pinned in, at most nine can be pinned, and Move Pin Up and Move Pin Down (Conversation menu and the row context menu) reorder them.
- Mark as Unread is stored on the account (room account data) and read back from the sync, so it shows on other devices too; opening the conversation clears it there as well.
- Emoji picker: 64 emoji, a search field (type words such as "party", Return picks the first result) and the recently used emoji first. The recent ones are saved in the app's data directory.
- Tapping the quoted line of a reply scrolls the timeline to the original message.
- Message context menu: Edit History lists the earlier versions of an edited message, and Show Reactions names who gave each emoji.
- Conversation details also list the pictures shared in the loaded messages.
- Not yet: end-to-end encryption, dragging files in, a user directory search in New Message.

## 0.2.0 — 2026-10-13

- Conversations: New Message (File menu, Cmd+N, or the + button) starts a direct conversation with a Matrix user id; Delete Conversation leaves and forgets a room after a confirmation; Pin keeps a conversation at the top; Hide Alerts stops its notifications and its dock badge count; Mark as Unread (also Mark as Read). The choices are saved in the app's data directory. All are in the Conversation menu and in the context menu of a conversation row.
- Search field above the conversation list (Cmd+F): filters by conversation name and by the text of loaded messages.
- Messages: reply (a quoted line in the bubble), edit and Undo Send on your own messages, tapbacks from the message context menu with reaction chips under the bubble (tap a chip to add or take back your own), and Copy. Edited messages are marked.
- Attach File sends any file, not only pictures. Dragging files in is not available yet.
- Details panel with the members and Rename; Settings (Cmd+,) shows the account and signs out, which ends the session and removes the saved login. Emoji picker for the composer (Edit menu or the Emoji button).
- Not yet: end-to-end encryption, dragging files in.

## 0.1.1 — 2026-10-13

- Timeline: a day heading (Today, Yesterday, or the date) where each day's messages start, and a typing line ("bob is typing...") at the foot of the conversation when others type. Typing is read from the sync; the app does not send its own typing state yet.
## 0.1.0 — 2026-10-13

- New app: Messages, a Matrix client. Sign in with a homeserver, user name and password (https only). The password goes
  to the shared passwords group (`is.bifrost.passwords`) as a login under `matrix:<homeserver>`, so Settings > Passwords
  lists it. Each start signs in again with the saved password.
- Conversations: one long-polling sync for the account; the room list is ordered by the newest message, with unread counts
  and a preview. Opening a room shows its timeline and marks the newest message read (a public read receipt).
- Sending: text messages, and pictures (PNG, JPEG, GIF or WebP, up to 25 MB) uploaded to the homeserver first.
- Notifications for messages from other people after the first sync, and the unread total on the dock badge.
- Not yet: end-to-end encryption (encrypted rooms show "Sent an encrypted message"), received pictures are listed by
  name only, invitations cannot be accepted, and only the first saved account is used.
