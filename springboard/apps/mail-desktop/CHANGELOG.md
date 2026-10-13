# Changelog

## 0.9.0 — 2026-10-13

- Message > Attach Files (Shift+Cmd+A) opens the file panel and adds the chosen files to the new message; the sheet
  lists them and Remove Attached Files clears the list. A message with files is sent as `multipart/mixed` with base64
  parts. Archive moves to Ctrl+Cmd+A.
- Message > Forward as Attachment sends the original message whole as a `message/rfc822` file.
- Save As, Save Attachments and a click on a file name in the reader now ask where to save through the save panel. Mailbox
  > Export Mailbox reads every message of the shown mailbox and writes an `.mbox` file.
- Mailbox > Edit Signature keeps one signature per account, added after a `-- ` line when a message is sent.
- Message > New Rule, Apply Rules (Opt+Cmd+L) and Remove Rule: a rule looks for text in From, To or Subject and moves,
  archives, trashes, junks, reads or flags the message. Rules also run on new inbox mail.
- Message > Remind Me (1 hour, tomorrow, 3 days, next week) keeps a message out of the list until its time, then brings
  it back as unread with a notification.
- Message > Add Sender to Contacts sends the sender's name and address to the address book.
- Mailbox > New Smart Mailbox saves a search that appears in the sidebar under the mailboxes; Delete Smart Mailbox
  removes it.
- View > Category lists only Primary, Transactions, Updates or Promotions mail, sorted from the message headers.
- Edit > Find in Message (Opt+Cmd+F), Find Next and Find Previous (Cmd+G) highlight matches in the open message and
  scroll to them.
- Reply All is on the toolbar.

## 0.8.0 — 2026-10-13

- Message > Flag Color (Red, Orange, Yellow, Green, Blue, Purple, Gray) and Clear Flag. The colour is kept in the
  `$MailFlagBit0-2` keywords next to `\Flagged`, and the list row shows a dot in the colour. Clearing a flag also
  clears its colour.
- Message > Mute (and Unmute): new mail in a muted conversation raises no notification. The muted conversations are
  kept in the app's data directory.
- View > Raw Source (Opt+Cmd+U) shows the whole message as the server holds it. View > Hide Sidebar and Show Sidebar
  (Ctrl+Cmd+S) give the room to the list and the reader.
- Message > Save As (Shift+Cmd+S) writes the raw `.eml` file into the data directory; Save Attachments moves to
  Opt+Cmd+A.
- Mailbox > Open Quickly (Shift+Cmd+O) goes to the mailbox whose name matches what is typed.

## 0.7.0 — 2026-10-13

- Compose has Cc, Bcc and Reply-To fields (View > Cc, Bcc and Reply-To Address Field show or hide them, in the open
  sheet too). Bcc addresses get the message but are named in no header; Reply-To is sent as a header. Message > Set
  Priority sends `X-Priority` and `Importance`.
- Message > Reply All (Cmd+Opt+R) answers the sender (or the Reply-To address) and everyone else, once each, without the
  user's own address; Cc stays in Cc. A plain Reply now goes to the Reply-To address when the message has one.
- Message > Forward (Cmd+Shift+F) quotes the original headers and text under a `Fwd:` subject.
- Message > Move to Junk, Move To and Copy To (a submenu of the account's mailboxes; Copy To uses IMAP COPY) and Move
  Again.
- Mailbox > New Mailbox, Rename Mailbox, Delete Mailbox (IMAP CREATE, RENAME, DELETE), Erase Deleted Items and Erase
  Junk Mail (STORE `\Deleted` then EXPUNGE).
- View > Sort By (Date Received, From, To, Subject; ascending or descending) and View > Filter (All, Unread, Flagged,
  Replied). View > Collapse or Expand Conversation, Expand All and Collapse All fold the threaded view to the first
  message of each conversation.
- Edit > Paste as Quotation (Cmd+Opt+V) puts `> ` before each pasted line in the open message.

## 0.6.0 — 2026-10-13

- Find (Cmd+F) searches the server: the shown mailbox is searched with IMAP `UID SEARCH` (from:, to:, subject:, body:
  terms, or any field for a plain word, every term ANDed; non-ASCII under CHARSET UTF-8), and the newest 50 matches'
  headers replace the list when they arrive. Until then the loaded rows that match are shown.
- New mail arrives over IDLE (RFC 2177): one watcher per account keeps a connection on the inbox and lists it when the
  server reports a message. A server without IDLE, or a refused password, leaves the account on the 60 s poll; a lost
  connection is opened again after a minute, and a watched account is checked by the poll every 10 minutes.
- Threads: a reply sits under the message it answers, indented by its depth (up to six levels), oldest reply first.
- Accounts saved by 0.3.0 move to the shared Internet Accounts list on the first run, once: the account and its
  password (from the old keychain item, saved in Passwords under the IMAP host and user name, then the old item is
  deleted). The old `accounts.ini` is renamed `accounts.ini.migrated`. An account already in the list (same address) is
  not added again. Nobody re-enters an account.
- Mail keeps at most 64 message texts in memory. The oldest are dropped first, and a dropped message is loaded again when it is opened, so a long session in one mailbox no longer grows without bound. The open message is never dropped.

## 0.5.0 — 2026-10-13

- Message > Print... (Cmd+P) prints the open message's headers and decoded body as plain text, through the shared print
  operation. The job is submitted on its own thread.

## 0.4.0 — 2026-10-13

- Accounts come only from the shared Internet Accounts list now: Mail reads and writes `internet-accounts.ini` through
  `bifrost_app::controls::internet_accounts` and no longer keeps its own `accounts.ini`. Passwords are in the Passwords
  group (`is.bifrost.passwords`), under the IMAP host and the user name, the same item Settings and Passwords show.
  Accounts saved by 0.3.0 are not migrated: add them again in Settings > Internet Accounts or in the Mail setup sheet.
- Internal: IMAP UID comments reworded; no behaviour change.
## 0.3.0 — 2026-10-13

- Live Mail: account setup sheet (password in the keychain), IMAP over TLS or STARTTLS for mailboxes, the newest
  headers and message bodies, SMTP submission with AUTH PLAIN, compose and reply sheets, and new-mail notifications
  from a 60 s inbox poll. The window is the three-pane layout over the existing model.
- Accounts come from the shared Internet Accounts list (`bifrost_app::controls::internet_accounts`): `mail_accounts` reads the
  accounts with the Mail service on. Address checks use the shared `valid_address`.
## 0.2.0 — 2026-10-13

- Safe HTML mail: `html` sanitizes bodies (scripts, styles, frames, event handlers and script or relative URLs removed,
  remote images blocked and counted, inline raster images kept) and gives plain text for previews and search.
## 0.1.0 — 2026-10-13

- New app: the mail core. Accounts (IMAP and SMTP servers, password kept in the keychain), a pure IMAP response parser and
  command builder, an SMTP reply parser and command builder, RFC 5322 compose with RFC 2047 subjects, flags (read,
  flagged, answered, draft, deleted), threads by References and In-Reply-To with a subject fallback, search with field
  terms, and the three-pane width layout. The window UI is not wired yet.
