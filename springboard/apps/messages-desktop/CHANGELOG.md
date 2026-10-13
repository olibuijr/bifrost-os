# Changelog

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
