# Mail: server search, IDLE, threads, first-run migration (slug mail-server-search)

Mail follow-up, version 0.6.0 (`springboard/apps/mail-desktop`). This is not a parity matrix row: it finishes three
Mail features the earlier waves left as models and adds the migration from 0.3.0. No reference macOS files were used.
The work follows the existing IMAP and SMTP wire code and the shared Internet Accounts list.

## Find: server-side SEARCH

- API: IMAP `UID SEARCH` (RFC 3501 section 6.4.4). The criteria come from `search::Query::imap_criteria`.
- Path: `src/search.rs` (`imap_criteria`, `term_criteria`, `any_of`), `src/session.rs` (`Session::search`),
  `src/sync.rs` (`search`, `Event::Found`), `src/state.rs` (`Mail::found`, `apply_found`, `row`, `visible_rows`),
  `src/window.rs` (`search_server`, the Find sheet submit).
- Done: field terms map to FROM, TO, SUBJECT and BODY; a plain word is an OR over the four fields; terms are ANDed;
  non-ASCII text sends `CHARSET UTF-8`. The newest 50 matches' headers replace the list when they arrive, and only for
  the same search, account and mailbox. Until then the loaded rows that match are shown.
- Needs Midget: a server with a large mailbox (BODY search speed), and a non-ASCII query on a real server.

## New mail: IDLE with poll fallback

- API: IMAP `IDLE` (RFC 2177), `CAPABILITY`, and `DONE`.
- Path: `src/session.rs` (`Session::idle`, `Session::capabilities`), `src/imap.rs` (`is_new_message`, `idle`,
  `IDLE_DONE`), `src/sync.rs` (`watch`, `watch_inbox`, `WatchEnd`, `Event::Watch`, `Event::Arrived`),
  `src/window.rs` (`refresh` starts a watcher once per account, `poll` skips a live account except every
  `IDLE_CHECK_ROUNDS`).
- Done: one watcher thread per account with a password. A server without IDLE, or a refused login, stops the
  watcher and the account stays on the 60 s poll. A lost connection is reopened after 60 s. Each new message (`n EXISTS`)
  lists the inbox again, which feeds the existing new-mail notices.
- Needs Midget: a real server's IDLE timeout (servers drop IDLE after about 30 minutes; the reconnect covers it), and
  the notification timing against the poll.

## Threaded tree indentation

- Path: `src/threads.rs` (`arrange`), `src/state.rs` (`Line::depth`, `visible_lines`), `src/draw.rs` (`INDENT_PT`,
  `MAX_INDENT_LEVELS`, the list loop).
- Done: each conversation is in tree order. A reply's parent is the nearest message it answers that is in the group
  (In-Reply-To first, then References from the end). Replies sit under their parent, oldest first; the depth indents
  the subject line by 14 pt per level, up to six levels. A reference cycle still lists every message once.
- Needs Midget: the look of the indent in both appearances.

## First run: 0.3.0 accounts move to Internet Accounts

- Path: `src/migrate.rs` (`run`, `plan`, `shared_text`, `legacy_name`, `LEGACY_SERVICE`), `src/accounts.rs` (`import`),
  `src/window.rs` (`new`, `keychain`, `moved`, `Pending::Migrate`, `Pending::Moved`, `Pending::Discard`), and
  `src/main.rs` (the `keychain` reply hook).
- Done: `accounts.ini` in the data directory is read once. Each section becomes a mail account (0.3.0 had no `kind`
  line). An account whose address is already a mail account in the shared list is skipped; a taken id gets `-2`, `-3`.
  The new accounts are written to `internet-accounts.ini` in one write. The password is read from the old keychain item
  (`is.olibuijr.mail.desktop`, `account:<old id>`), saved in Passwords under the IMAP host and user name, and the old
  item is deleted. A missing old item falls back to Passwords. The old file is renamed `accounts.ini.migrated`, so it is
  never read again. A damaged old file is reported in the status line and left in place.
- Needs Midget: a machine that really has a 0.3.0 `accounts.ini` and keychain items, to check the password moves and the
  old item is removed.

## Migrations and deletions

- No superseded code was removed in this slice. The 0.4.0 unify already deleted the private account model.
- `Line` gained `depth`; `Mail::flags_of` and `set_flag_locally` now also see the server-found rows.

## Tests

Unit tests cover the IMAP criteria, the IDLE and CAPABILITY exchanges on the scripted line, the threads tree, the
server-answer rules in state, and the migration plan (dedupe by address, id renaming, damaged file). Nothing was built
or run in this round: the lead builds and runs `cargo test -p bifrost-mail`.
