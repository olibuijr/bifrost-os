# Mail (round 75 slice)

Slice: `apps/mail-desktop` (crate `bifrost-mail`, library `bifrost_mail`).

## Status

The window, network and account UI are not wired yet. The binary still exits with a message. This round adds the
safe HTML body path to the core. Nothing here was built or run: the lead builds and tests.

## Done this round

| Area | Where | What |
|---|---|---|
| Reading HTML mail safely | `springboard/apps/mail-desktop/src/html.rs` (`sanitize`, `to_text`) | Removes scripts, styles, frames, objects, templates and comments with their content. Keeps a fixed tag whitelist and drops other tags but keeps their text. Keeps only `href` for http, https and mailto, and `src` for remote images (blocked and counted unless allowed) or inline png, jpeg, gif and webp `data:` images. Keeps `alt`, `title`, numeric `width`, `height`, `colspan`, `rowspan`. `to_text` gives a plain-text body with line breaks and decoded entities. |

Reference used: none for this item. It follows general HTML mail safety rules, not a macOS reference file. Mail.app
resources under `~/Projects/macos-ref/extract/ref27/` were not measured this round.

## Still open (not started)

- Account setup with credentials in the keychain or passwords store: the core `account.rs` has the model; no UI or keychain call.
- IMAP mailboxes and sync: the parser and command builder exist in `imap.rs`; no connection or sync loop.
- Threaded message list: `threads.rs` groups messages; no list view.
- Compose, reply, forward with attachments over SMTP: `message.rs` and `smtp.rs` build messages; no attachments, no send loop.
- Search UI: `search.rs` parses terms; no search field.
- Flags UI and new-mail notifications: `flags.rs` only.
- Mail.app menus and message list columns: needs REA measurement of `ref27/` Mail.app resources (see brief).

## Needs Midget checking

- Rendering of sanitized HTML is not yet on screen, so nothing is checked on Midget.
