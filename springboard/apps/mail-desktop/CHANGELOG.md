# Changelog
## 0.4.0 — 2026-10-13

- Accounts come only from the shared Internet Accounts list now: Mail reads and writes `internet-accounts.ini` through
  `bifrost_app::controls::internet_accounts` and no longer keeps its own `accounts.ini`. Passwords are in the Passwords
  group (`is.bifrost.passwords`), under the IMAP host and the user name, the same item Settings and Passwords show.
  Accounts saved by 0.3.0 are not migrated: add them again in Settings > Internet Accounts or in the Mail setup sheet.
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
