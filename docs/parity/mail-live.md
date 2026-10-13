# Mail live (round 75, slice mail-live)

App: `springboard/apps/mail-desktop` (bifrost-mail 0.3.0, id `is.olibuijr.mail.desktop`).

## Evidence used

Reference bundle (clean room, names and property lists only; nothing copied into the repo):

- `~/Projects/macos-ref/extract/ref27/System/Applications/Mail.app/Contents/Info.plist`
  - `CFBundleURLTypes`: scheme `mailto` (Email Address URL), `message`, `mail-pref-pane`. The app does not register
    schemes yet (see gaps).
  - `CFBundleDocumentTypes`: Email Message (`com.apple.mail.email`) as a viewer, plus mailbox types.
  - `LSMinimumSystemVersion` 27.0.
- `.../Mail.app/Contents/Resources/Base.lproj/*.nib` (names only): `AccountInfo.nib`, `ComposeWindowController.nib`,
  `ComposeViewControllerWK2.nib`, `MailboxesViewController.nib`, `MailboxesContextMenu.nib`,
  `ConversationViewController.nib`, `ConnectionDoctor.nib`, `DeliveryFallbackPanel.nib`, `IMAPQuota.nib`,
  `HyperlinkPanel.nib`. These set the structure: an account setup sheet, a compose window, a mailboxes sidebar,
  a conversation view, and delivery-failure feedback.
- REA decode of the nibs was not run in this round (time). The menu shortcuts below are the app's choice, not
  read from the nibs.

## Rows

| Entry | API / reference | BifrOSt path | Done | Needs Midget |
|---|---|---|---|---|
| apps/Mail (`/System/Applications/Mail.app`) | app | `springboard/apps/mail-desktop` | Live window: sidebar of accounts and mailboxes, newest-first message list, reader, toolbar (Compose, Reply, Refresh), account setup sheet, compose and reply sheet, menus (Message, Mailbox) | Yes: real IMAP and SMTP servers, layout at phone-size and wide windows, Dark and Light |
| (supports) IMAP over TLS | RFC 3501 | `src/session.rs`, `src/imap.rs`, `src/line.rs` | LOGIN, LIST, SELECT, UID SEARCH, UID FETCH headers and body, STORE Seen; literals folded into quoted strings; STARTTLS | Yes: a server with TLS on 993 and STARTTLS on 143 |
| (supports) SMTP submit | RFC 5321 / 4616 | `src/submit.rs`, `src/smtp.rs` | EHLO, STARTTLS on 587, AUTH PLAIN, MAIL, RCPT, DATA with dot-stuffing, QUIT | Yes: a real send and a refused login |
| (supports) TLS transport | shell rustls client | `springboard/src/deildu/http.rs` (`Transport::upgrade`, default method) | Shared client; STARTTLS upgrade implemented for `Tls` | Yes |
| (supports) passwords | keychain | `src/window.rs` (`Pending`), `src/account.rs` | Add writes the password to the keychain (update on duplicate); start-up reads each account's password; nothing written to `accounts.ini` | Yes: restart the app and check the password loads |
| (supports) new mail | user notifications | `src/window.rs` (`notify`), `src/state.rs` (`arrivals`) | 60 s inbox poll per account; first listing sets a baseline; newer UIDs post "New Mail" with sender and subject | Yes: send a message to the account and wait |
| (supports) message bodies | MIME (RFC 2045/2046/2047) | `src/body.rs`, `src/message.rs`, `src/html.rs` | Plain part preferred over HTML; nested multipart; base64 and quoted-printable; ISO-8859-1 and UTF-8 charsets; HTML to text | Yes: a multipart HTML message, an Icelandic subject |

Closed entry for the lead: `apps/Mail` (gap) with the path above. `frameworks/MailKit` stays a gap (no extension
points yet).

## Not done (honest gaps)

- The list is flat. `src/threads.rs` and `src/search.rs` are pure and tested but not wired into the window.
- No IDLE: new mail arrives on the 60 s poll.
- No delete, archive, flag or move actions; no mailbox create or rename; no attachments.
- No account edit or removal; the setup sheet only adds.
- HTML is shown as text (`html::to_text`), not rendered; remote images are never loaded.
- `Security::Plain` is not offered in the setup sheet (ports 993 and 465 use TLS, the others STARTTLS).
- Menu shortcuts: Cmd+N (New Message), Cmd+Shift+R (Reply), Cmd+Shift+D (Send), Cmd+R (Refresh). Check the Send
  shortcut against the Mail menu nib before calling it a match.
- Nothing here has been compiled in this round (wave rule: the lead builds). The pure parts carry unit tests with
  scripted connections (`line::script`).

## Shared-file edit

`springboard/src/deildu/http.rs`: a default `Transport::upgrade` method and its `Tls` implementation (about ten
lines). Existing transports compile unchanged.

## Cargo

`apps/mail-desktop/Cargo.toml` now depends on `bifrost-app` and `springboard` (path deps, same as the assistant app).
`Cargo.lock` changes when the lead builds; no new registry crates.
