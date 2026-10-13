# Changelog

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
