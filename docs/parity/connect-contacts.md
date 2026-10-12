# Connect: Contacts to the pasteboard and the search index

Status: wired in the app, not built or run. This round ran no cargo, rustc or tests (the lead builds once). The host
tests in `springboard/apps/contacts-desktop/src/services.rs` and `contacts.rs` are written but unexecuted.

Branch `connect/contacts`, app `bifrost-contacts` 1.3.0 (version not bumped this round; CHANGELOG 1.3.0 extended).

## What is connected

| Service | Feature | Matrix slug, feature | Where in the app |
|---|---|---|---|
| `pasteboard` (general board, `write`) | Copy a contact as a vCard and as plain text | `application-services`, "Pasteboard (NSPasteboard)"; `core-transferable`, "Copy and paste of typed items" | `src/services.rs` (`contact_item`, `copy_contact`), `src/contacts.rs` (Target::CopyContact, `take_copy`), `src/main.rs` (`after_input`, `service_reply`) |
| Core Search index (`CSSearchableIndex`, read by the `search-index` ranking) | Each contact indexed by name, company, phone numbers and emails | `core-spotlight`, "Searchable items and attribute sets" | `src/services.rs` (`search_item`, `SearchIndex::sync`, `plan`), `src/main.rs` (`configure`, `after_input`) |

### Details

- **Copy Contact.** A new button on the contact card (under Share Contact, node id 204, title "Copy Contact"). It
  queues the open contact; `main.rs` sends one `write` to the `pasteboard` service on the `general` board. The item has
  two representations: `public.utf8-plain-text` first (name, company, phone, email, address, one per line, so a text
  field takes it), then `public.vcard` (a vCard 3.0 card from the app's own `exchange::export`).
- **Status line.** The write's reply is handled in `App::service_reply`. A success shows "Copied <name> as vCard and
  text"; an error (for example an older shell answering "no service pasteboard") shows "Could not copy <name>". The
  app never reports success it did not get.
- **Search index.** Each contact is an item with id `contact.<id>` in the domain `is.olibuijr.contacts.desktop`. The
  title is the contact's label (name, or the first of company, phone, email). Keywords are the name, company, phone
  and email; the description is phone and email. The first sync of a run deletes everything the app indexed before and
  indexes all contacts. Later syncs compare a fingerprint (name, company, phone, email) with what was sent: only new or
  changed contacts are indexed, and removed ones are deleted. Nothing is sent when nothing changed.
- **When it is called.** Pasteboard: only on a Copy Contact press. Index: once at launch (the first `configure`), then
  after each input event (`after_input`). No call is made per frame, and a resize does not re-index.

## How a user sees it

1. Open Contacts, pick a contact, and press Copy Contact on the card. The header shows "Copied <name> as vCard and
   text".
2. Paste (Command-V or the paste key) into a text field in another app: the plain text (name, phone, email, address)
   comes out.
3. Open the desktop search and type the person's name, a word of their company, a phone number or an email address.
   The contact is listed in the Documents section, even when the search words are in another order.

## How to verify on the test PC (Midget)

Build and publish go through the lead (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`).

1. Open Contacts: `bifrost pc midget open-app is.olibuijr.contacts.desktop`, then `bifrost pc midget state`.
2. Select a contact with a phone and an email. Press Copy Contact (`bifrost pc midget tap-window APP X Y`, window-local
   pixels, not raw `tap`). Expect the header line "Copied <name> as vCard and text".
3. Paste into a text field of another app (for example the search field of the desktop search, or a note). Expect the
   plain text, one field per line.
4. Check the pasteboard service directly from an app: a `read {"board":"general"}` call through the `pasteboard`
   service (see `docs/parity/wire-svc-pasteboard.md`) returns one item with the two representations.
5. Index check: with Contacts open, search the desktop search for a contact's phone number. The contact appears. Then
   edit the contact's phone in the app, and search again: the new number finds it, the old one does not.
6. Launch check: quit and reopen Contacts. The first sync clears and re-adds the items, so the search results do not
   double.
7. Screenshot with `bifrost pc midget screen`; the new card row sits between Share Contact and Delete Contact.

## Gaps

- **Cargo.lock.** `bifrost-contacts` gained the `bifrost-core` dependency (for the `service_reply` type). The lock
  file at the repo's workspace root is not edited in this branch; the lead's build updates it. A `--locked` build fails
  until the lock is regenerated.
- **Shell vCard type.** `public.vcard` is not in the shell's type model (`springboard/ui/src/uttype`), so the shell does
  not map it to a MIME type and does not promise to keep the vCard representation on the general board. Only the plain
  text is confirmed to reach the clipboard; the vCard is sent but not yet confirmed to be read by the shell.
- **Search results do not open the contact.** The app does not implement `continue_activity`, so choosing a contact in
  desktop search opens the app without selecting it. The index is one-way in this round.
- **No ranking through `search-index`.** The app's own search field still filters its list locally. The
  `search_ranking` client (`bifrost_app::search_ranking`) is not used, because the index is fed through Core Search.
- **Index limits.** The shell keeps at most 1000 items per app, and long text is cut. A contact has one phone and one
  email, so the index holds at most a few keywords per contact. Items over the wire identifier cap are refused by the
  SDK and not sent; contact ids are short, so none is.
- **Copy requests are not queued.** `take_copy` runs after every input, so each press is sent at once. Two presses
  before a reply each get their own reply line (the copies map is keyed by request id).
- **Compliance snapshot.** The card has a new button, so the compliance golden in `tests/snapshots/` may need
  `BIFROST_BLESS=1` after a host run. Not regenerated here.
- **Share Contact is still a no-op.** The share sheet is a shell feature not available to packages (the existing
  comment in `main.rs`). Copy Contact is the working path.
- **Not run on a device.** The Midget steps above are the plan, not a result.
