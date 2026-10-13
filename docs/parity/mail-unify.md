# Mail: unified accounts, threads, search, flags, archive and attachments (mail-unify)

Slice: the Mail app (`springboard/apps/mail-desktop`, bifrost-mail 0.4.0). Not a reference-SDK API row. The work
unifies the account model and finishes the mail window. Evidence: the code and the existing parity docs
(`docs/parity/internet-accounts.md`, `docs/parity/accounts.md`). The macOS
reference extracts were not used in this round.

## Accounts (done, one model)

- Mail's accounts are `bifrost_app::controls::internet_accounts::Account` with `Kind::Mail`, read and written only
  through the shared list (`internet-accounts.ini`, `shared::load` / `shared::save`).
- Removed: `springboard/apps/mail-desktop/src/account.rs` (private account model and file format) and `src/store.rs`
  (private `accounts.ini` store). Callers migrated: `session.rs`, `submit.rs`, `transport.rs`, `sync.rs`, `state.rs`,
  `window.rs`.
- `accounts.rs`: `load_all`, `load_mail` (Mail service on), `add` (appends to the shared list, refuses a duplicate
  id), `incoming` and `outgoing` (the IMAP and SMTP `Server`s, which are `Option` in the shared model).
- Passwords: the passwords group (`bifrost_app::passwords`, group `is.bifrost.passwords`), service = `password_site()`
  (IMAP host), account = `username`. Read at start with `passwords::fill`, saved with `passwords::save` (`update` on
  a duplicate). Answers come back in `MailWindow::service_reply` (`App::service_reply` in `main.rs`), replacing the
  keychain calls. Manifest: `keychain_groups = ["is.bifrost.passwords"]`.
- The new account id is unique among the whole shared list, not only Mail's accounts.
- Migration: none. Accounts saved by 0.3.0 in `accounts.ini` are not read; the user adds them again (CHANGELOG 0.4.0).

## Flags, read state, archive, trash (done in code)

- Reader and toolbar: Archive and Trash buttons when a message is open; Message menu: Flag (cmd+shift+l), Mark as
  Unread, Archive (cmd+shift+a), Move to Trash.
- Flags change the row at once (`set_flag_locally`), then `STORE ... .SILENT` runs in the background (`session::set_flag`).
- Move: `UID MOVE`, with the fallback `UID COPY`, `\Deleted`, `UID EXPUNGE` for servers without MOVE (`session::move_to`).
  The target is found among the names providers use (`state::ARCHIVE_NAMES`, `TRASH_NAMES`). A refused change
  refreshes the listing and shows the server's text.

## Threads view (done in code)

- Mailbox > Show Threads (cmd+shift+t). `Mail::visible_lines` groups the matching messages with `threads::group`;
  each conversation is listed together, newest first, and its subject shows `(n)`.

## Search (done in code, local only)

- Mailbox > Find... (cmd+f) opens the Find sheet (`Sheet::Search`); the terms are the `search` module's (`from:`,
  `to:`, `subject:`, `body:`, quoted phrases). Show All Messages clears it.
- Matches the loaded headers, and the body where it is loaded. The server is not asked (no IMAP `SEARCH` yet).

## Attachments (done in code)

- `body::attachments` walks the MIME structure (nested multipart included) and returns the file parts (a name, or an
  attachment disposition), base64 or quoted-printable decoded. The reader lists them under the headers with sizes.
- Mailbox > Save Attachments... (cmd+shift+s) writes them to `<data dir>/attachments/<uid>/` (`attachments::save_all`).
  A repeated name becomes `2-name`.

## Still needs checking on Midget

- Everything compiles only after the lead builds (no build was run in this round, per the brief).
- Real servers: UID MOVE and the COPY fallback, the Seen/Flagged round trip, the attachment save path under the app's
  storage permission, and the Find sheet's keyboard focus.
- Thread grouping of messages without threading headers (subject fallback) on a real mailbox.
- Matrix: the `Mail` app row (`docs/parity-matrix/os/apps/m-o.json`, status gap) is a candidate to close once Midget
  confirms the window. The lead closes it; this slice does not edit `docs/parity-matrix/`.
