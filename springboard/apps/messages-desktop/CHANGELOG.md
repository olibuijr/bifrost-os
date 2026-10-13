# Changelog

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
