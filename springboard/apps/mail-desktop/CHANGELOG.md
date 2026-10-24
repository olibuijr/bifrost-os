# Changelog

## 0.17.5 — 2026-10-24

- Add to Contacts saves the sender in the contacts service's new save format, where phones and emails are labeled
  values (`{label, value}`), so the contact is saved by the system release that brings that format. Built with it.

## 0.17.4 — 2026-10-24

- Message > Mark as Unread on the open message shows its unread dot at once (white on the selected row), and the
  message stays unread while it is open; it is read again only once another message is shown and it is opened again.
  The dot was hidden on the selected row and showing the message again marked it read.

## 0.17.3 — 2026-10-24

- Switching mailboxes shows the mailbox's last listing at once, with the message it had selected, and the new listing
  refreshes it in place: the list was emptied to "Loading..." for several seconds on every switch, even for an inbox
  listed moments before. Only a mailbox never listed in this run shows Loading. Each account keeps its own rows.
- Message viewer windows open cascaded beside the main window instead of over its message list.

## 0.17.2 — 2026-10-24

- Clicks in the main window land where they are aimed again after a compose window closes (sent, saved, discarded or
  closed): the main window took back its own size only on some of those paths, so a sidebar click could select a
  message of the list.
- Why a message was not sent (an address that is not valid, no password, the server's answer) shows in its compose
  window, over the buttons, instead of in the main window's footer; an edit clears it.
- The compose form fills its window, and the message field grows with the window.

## 0.17.1 — 2026-10-23

- Rebuilt with the shared text-rendering fix so invisible formatting characters in message viewers no longer appear as missing-glyph boxes.

## 0.17.0 — 2026-10-23

- The compose window takes clicks, typing, Esc and its menu items (keys and commands now reach the window they were sent to, and its fields hit-test at the window's own size).
- Up and Down move through the message list; Shift-click, Cmd-click and Shift-arrows select several messages, and Archive, Trash, Junk, Move, Flag and Mark as Unread act on all of them.
- A right-click on a message opens its actions; a double-click opens it in a viewer window.
- Each mailbox remembers its selected message; toolbar actions stay in place disabled instead of disappearing; the unread dot has a gap before the sender.
- The viewer and compose windows scroll with the wheel, at the right speed on scaled screens.

## 0.16.2 — 2026-10-20

- Reading hostile mail no longer hangs or crashes Mail: HTML tags are parsed in linear time (a tag is at most 4 KB, and
  an unfinished tag shows as text), HTML input is capped at 4 MB, and multipart messages are followed at most 12 levels
  deep and 2000 parts wide.
- Servers named by a domain's autoconfig file are trusted only when the file came from the address's own domain (a
  redirect to another domain does not count) and names a server in that domain; any other server is shown for
  confirmation before the password goes there.
- A Subject, In-Reply-To or References value can no longer start another header (line breaks are removed), and a lone
  CR in a message body is a line end, so it can no longer end the SMTP data early.
- The inbox watcher really idles: the socket's read timeout no longer ends IDLE, IDLE is renewed after 25 minutes, and a
  watcher stops when its account is removed or its settings change (and starts again with the new login).
- A Passwords item shared by two accounts with the same server and user name is kept when one of them is removed or
  edited, and adding an account does not overwrite another account's saved password.
- An account set to no encryption connects only to this computer or a private network.
- After the message is handed to the SMTP server Mail waits up to five minutes for the answer; with none, the message is
  not sent again by itself (it may have been delivered) and the person is told to check Sent Items.
- The message cache is limited to 96 MB as well as 64 messages, a mailbox export is limited to 256 MB, and a message
  literal is quoted without an extra copy.
- Remote images are counted in the decoded HTML part of a message, not in the raw text.
- The new-mail baseline follows UIDVALIDITY (a renumbered mailbox announces nothing old) and starts on an empty inbox.
- Mail's DNS client asks the system's name servers only (no public resolver), retries after a failure, does not stay
  disabled after a SERVFAIL or REFUSED, uses random transaction ids and follows a truncated reply over TCP.
- A damaged outbox file is kept as `mail-queue.json.bad` with a note in the status line instead of being overwritten, and
  saved attachment names stay unique.

## 0.16.1 — 2026-10-20

- The New Mail banner is built in one place for the window and the background check, and a test holds that its sender and
  subject are the decoded ones (never `=?utf-8?Q?...?=`).

## 0.16.0 — 2026-10-20

- Add Account asks for a name, an address and a password. The servers are found from the address: the domain's autoconfig
  file (`autoconfig.<domain>`, then `.well-known`, over https), then the DNS service records of RFC 6186
  (`_imaps`, `_imap`, `_submissions`, `_submission`), then the domain's own mail exchanger, and `imap.` and `smtp.` of the
  domain only when they exist. The sheet says what it is looking up and why when nothing is found. Servers that only DNS
  names, and that are outside the address's domain, are shown for confirmation before the password goes to them.
- Configure Manually (click or Space) shows the server fields for an account that is set up by hand.
- Senders, recipients, subjects and file names are read properly from RFC 2047 encoded words: B and Q encodings in any
  case, UTF-8, US-ASCII, ISO-8859-1, ISO-8859-15 and Windows-1252, adjacent words joined, a word that is malformed left
  as written. File names also read the RFC 2231 forms (`filename*=utf-8''...` and continuations). A name that decodes
  to one with a comma stays one recipient.
- Message text from HTML decodes every named and numeric character reference (`&bull;`, `&#8202;`, `&#x2022;`); the
  invisible padding characters senders put in the preview line are dropped.
- Declared charsets other than UTF-8 are read for message bodies too (Windows-1252, ISO-8859-15).
- Emoji in subjects and text draw in colour: the emoji font of the system is found on a PC (see the system CHANGELOG).

## 0.15.6 — 2026-10-20

- The Edit menu shows Select All, Copy, Cut and Paste enabled only while they can act: Select All with text in a sheet's field,
  Copy and Cut with a selection in a field that is not a password, Paste in a field that takes typed text (or the find bar).
- Typing or pasting over a selected message body no longer keeps the bold, italic, underline or colour of the text it replaces.

## 0.15.5 — 2026-10-20

- Select All (Cmd+A) in a sheet's field selects the field's whole text, and typing, Paste or Backspace replaces it, so an Add
  Account form can be retyped from the start. Copy and Cut take the selected text to the clipboard (a password is never
  copied or cut). Option+Backspace deletes the last word of the field and Cmd+Backspace the whole text.

## 0.15.4 — 2026-10-20

- With no account the Favorites rows, Inbox included, are greyed and do nothing; they come alive once an account is added.

## 0.15.3 — 2026-10-20

- The title "Mail" starts clear of the window buttons instead of running under them, and the toolbar starts after them too.
- The toolbar buttons are symbols (New Message, Get Mail, Reply, Reply All, Archive, Move to Trash) in capsules, not text buttons.
- The sidebar has Favorites (Inbox, Drafts, Sent, Junk, Trash) and Mailboxes with a symbol for each mailbox; with no account the rows are greyed and Mailboxes says there are none.

## 0.15.2

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.15.1

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.15.0 — 2026-10-17

Drafts:

- Drafts go to the server's own Drafts mailbox when it is flagged `\Drafts` (RFC 6154), and to the fixed Drafts name
  otherwise.
- Message > Edit Draft (in the Drafts mailbox) reopens a saved draft in a compose sheet with its addresses, subject,
  text, files and threading headers. Saving it again replaces the saved copy.

HTML mail:

- Inline `cid:` images resolve to the message's own picture parts in the sanitiser. Scripts, forms and event handlers
  stay removed and remote images stay blocked. The reader does not draw images yet.

## 0.14.0 — 2026-10-17

- Core Search finds the subjects and senders of the newest messages of the mailbox on screen (up to 200). Bodies are never indexed. Mail Settings "Show in Search" (on by default) turns it off and removes what was indexed. A result opens the message.

## 0.13.0 — 2026-10-17

Viewer windows:

- Double-click on a message row, or File > New Viewer Window, opens the message in a window of its own. The viewer is
  read-only: it shows the header, the files and the body of the copy the main window fetched, and it scrolls.

Compose windows:

- A new message, reply, forward or mailto link opens in a window of its own, and several can be open at once. Sending
  or cancelling closes the window.

Mail links:

- Mail opens `mailto:` links (manifest `schemes = ["mailto"]`). A new message opens with the To, Cc, Bcc, Subject and
  Body of the link filled in. The SDK call `bifrost_app::mail_compose::compose` hands a draft to the default mail app,
  with the files it names as read grants; Mail attaches them. A file named by path is refused.

Privacy:

- Remote content is blocked by default. The reader shows a note with Load Remote Content for the open message, and Mail
  Settings has a "Load remote content" switch, off by default.

Signatures:

- Mail Settings has "Signature above the quoted text": a reply's signature goes before the quote.

Drafts:

- Closing a message with changes asks Save This Message, Don't Save, or Cancel. Changed messages are also saved every
  30 seconds. Drafts go to the Drafts mailbox over IMAP, or to a file on this computer when the account cannot sign in.
  Sending removes the draft.

Compose windows:

- Send, Cancel and keys act on the compose window they came from. While a send is on its way, or a question is open, the
  window stays the active one.

Rules:

- Mailbox > Rules lists the rules, with a name that opens its editor, Add Rule, and Remove.

## 0.12.2 — 2026-10-14

- Draw and hit-test the mailbox toolbar and sidebar with the shared Golden Gate widgets.
- Move message-specific writing and search commands into a Writing menu to avoid duplicating the shell Edit menu.

## 0.12.1

- Removed the duplicate Edit menu; message specific text actions now appear in Message. Added the SDK and design compliance test harness.
## 0.12.0 — 2026-10-14

Reminders:

- Message > Ignore Follow-Up drops the reminders of the open message, so it does not come back on its own. The item is
  offered only while a reminder is waiting for the message.

Reading:

- Mail Settings has a "Bold unread messages" switch (on by default). Unread senders are drawn in semibold and read ones
  in regular type; with the switch off every sender is regular.

Removing attachments:

- Message > Remove Attachments leaves a line "[Attachment removed: name]" where each file was, so the text still says
  what the message carried.

Parity records:

- The mail matrix rows cite the modules the window calls (schedule, findbar, category, prefs, mime). The header-block
  models in organize.rs are not called by the window yet and are still covered by the headless tests.

## 0.11.0 — 2026-10-14

Testing and parity records (no change in the window):

- tests/parity_app-mail.rs covers the signature model, mailbox file reading and writing (From-line escaping, file and
  mailbox names), the body find with Icelandic capitals, categories, and the Remind Me times and due queues, headless.
- The mailbox import and export rows now cite the code that calls mbox::read and mbox::write.

## 0.10.0 — 2026-10-14

Reading:

- View > All Headers shows every header line above the text of the open message. The choice is remembered.
- The Reply All button and Message > Reply All are offered only when the message has recipients besides the sender and you.
- Messages sent with high or low priority carry a "High priority" or "Low priority" tag in the list.
- The toolbar shows a chip with the filter in use (Unread, Flagged, ...) while a filter is applied; a tap moves to the next
  filter, and All takes the chip away. The menu still holds every filter.

Writing:

- Replies start with the original quoted under an "On <date>, <name> wrote:" line. Mail Settings has a "Quote the original in
  replies" switch (on by default).
- Mail Settings has a "Plain text messages only" switch: such messages are sent without the HTML part.
- A reply is sent from the address the original was sent to when that address is one of the account's aliases.
- Message > Forward carries the files of the original along, within the limits of a message (8 files, 20 MB).
- Message > Paste and Match Style (Edit menu) pastes the text in the style of the text typed so far.

Saving:

- Message > Save Attachments asks for one folder and writes every file of the message into it, under the names the files
  had. Without a folder panel the files go into the app's data directory.
- Message > Export as PDF writes the subject and the text of the message to a PDF file through the save panel.

Finding:

- Open Quickly lists the mailboxes its field matches under the field, the best match first. A click opens one of them;
  Return still opens the first.

## 0.9.0 — 2026-10-13

Sending and receiving files:

- Message > Attach Files (and an Attach button on the compose sheet) opens the open panel; the files are listed on the sheet
  with a Remove button and sent as `multipart/mixed` (at most 8 files and 20 MB). The app now asks for the `documents`
  permission.
- Message > Forward as Attachment sends the original as a `message/rfc822` file. Message > Redirect sends the original
  unchanged with `Resent-` headers to new recipients.
- The files of an open message are listed one to a line in the reader; a tap saves that file through the save panel.
  Message > Save Attachments asks for each file's place in turn, and File > Save As uses the save panel too. Without the
  panel the files go into the app's data directory as before.
- Message > Remove Attachments writes the message again without its files (IMAP APPEND) and deletes the old copy.
- Message > Add Sender to Contacts saves the sender's name and address through the contacts service, unless the address
  is already there.

Writing:

- A Format menu: Bold, Italic, Underline, Text Color, Bulleted List, Increase and Decrease Indent, Clear Formatting. They
  apply to the text typed after them; a styled message is sent as plain text and HTML alternatives.
- Edit > Check Spelling underlines the misspelled words through the system text service. Next Misspelling shows
  suggestions and Correct Spelling replaces the word in view.
- Mailbox > Mail Settings keeps a signature and other send-from addresses for each account and an undo-send delay of up
  to 30 seconds. With other addresses the compose sheet has a From field that a click cycles through. Message > Undo
  Send takes a held message back.
- Message > Send Later adds a "Send in" field (30m, 2h, 1d); Mailbox > Send Queued Messages Now sends the outbox at once.
  Message > Remind Me brings a message back unread, with a notification, at the time given. Both survive a restart.
- The Cc, Bcc and Reply-To fields of new messages are remembered between runs.

Organising:

- Mailbox > New Smart Mailbox saves a search under a name; it is listed in the sidebar. A favourites bar (Mailbox >
  Add to Favorites Bar, View > Show Favorites Bar) pins mailboxes under the toolbar.
- Mailbox > Rules: New Rule (from, to and subject contain; move to a mailbox, mark read, flag in a colour), Delete Rule and
  Apply Rules. Rules also run on mail that arrives in an inbox, and a message a rule moves or reads raises no
  notification.
- View > Category shows Primary, Transactions, Updates or Promotions only. View > Filter adds With Attachments, View >
  Sort By adds Flag and Read Status. A Reply All button joins the toolbar.
- Conversations fold and unfold from a triangle in the first row. Muted conversations carry a Muted tag in the list.
- Message > Move to Junk marks the message `$Junk`; moving it out of Junk sets `$NotJunk`.
- Mailbox > Export Mailbox and Import Mailbox read and write mbox files through the save and open panels.
- Mailbox > Account Settings edits the shown account (servers and ports included) and Remove Account deletes it with its
  password.
- Edit > Find in Message opens a find bar over the reader: matches are highlighted, Return and Find Next step through
  them.
- Mailbox > Show Activity lists what Mail did lately; Mailbox > Check Connection signs in to both servers of the shown
  account and reports each answer.
- Move To and Copy To moved into a Move menu, which lists the first 22 mailboxes: one menu holds at most 48 items, and
  the Message menu had started to lose its last entries on accounts with many mailboxes.

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
