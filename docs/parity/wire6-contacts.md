# Wire 6: Contacts (several phones, emails and addresses per contact)

Release 1.7.0 of the Contacts app (`springboard/apps/contacts-desktop`, package `bifrost-contacts`). This round makes a
contact hold more than one phone number, email and address, each under a predefined label, and makes the list search
and the vCard exchange use them. The store format in `springboard/src/shell/contactstore.rs` is unchanged: it still
keeps one phone, one email and one address per contact. The extra values live in a new file beside it, which the app
owns (`values.txt`), so no file outside `springboard/apps/contacts-desktop/` and this note was touched.

## What is now wired

| Feature | What the user can do | Where in the app |
|---|---|---|
| Several labelled values per contact | A contact's card lists its store phone, then its extra phones, then its store email and extra emails, then addresses, each under its label (Mobile, Home, Work, Other; Home, Work, Other for emails and addresses). Up to six extra values per contact. | `src/extras.rs` (`Extras`, `Extra`, `ValueKind`, predefined labels from `contactstore::LABELS`); `card_entry` and `card_rows` in `src/contacts.rs`. |
| Migration of the old store | A contacts file written by 1.6.0 (or earlier) opens unchanged. Without `values.txt` a contact has only its store values. Once a value is added (by vCard import), `values.txt` is written next to `contacts.txt`. | `extras::load_checked` (missing file = no extra values); the store keeps its single values as before. |
| Search across all fields | Searching the list finds a contact by a store field (name, phone, email, notes, company, address) or by any extra phone, email or address. | `list_hits` in `src/contacts.rs` (used by `rebuild`). |
| vCard import keeps extra values | File > Import vCards (and the toolbar Import) keeps each card's second and later phones and emails under their TYPE labels, up to six per contact. Values past the limit are counted in the status line. | `exchange::import` (`contact_of` returns the extra values), `import_vcards` in `src/contacts.rs`. |
| vCard export writes extra values | File > Export vCard 3.0 and 4.0, and the toolbar export, write every phone, email and address of a contact, the store value first. | `exchange::export(contacts, extras, version)`; `card_of` and the address lines. |
| Copy Contact carries extra values | Copy Contact puts the extra values in the text (after the store fields) and in the vCard. | `services::plain_text`, `services::contact_item`, `services::copy_contact`; `take_copy` returns the contact with its extra values. |
| Delete drops extra values | Deleting a contact removes its extra values from `values.txt`. Values of contacts missing from the store are dropped on load. | `remove_selected`; `Extras::remove`, `Extras::retain` in `load`. |
| Card copy by row | Pressing a card row copies that row's value, store or extra. The accessibility control names the value (phone, email, address). | `Target::Copy(row)` in `activate`; `controls()` nodes "Copy phone" and so on. |

Other changes:

- `src/main.rs`: `mod extras;`. The Copy Contact path passes the extra values (`take_copy` returns `(Contact, Vec<Extra>)`).
- `Cargo.toml`, `manifest.toml`: version 1.7.0. `CHANGELOG.md`: 1.7.0 entry at the top, in plain language.
- The main content area already fills with `CONTENT_BG`; no `DETAIL_BG` fills remain in the app.
- The name guard (`python3 tools/check-names.py`) passes on this tree.

## How a user reaches it

1. Import: put a vCard with several phones or emails (for example one `TEL;TYPE=work` and one `TEL;TYPE=home`) in the
   Contacts Import folder and choose File > Import vCards. The status line names the count; the extra values are kept.
2. Open that contact: the card shows the store phone, then each extra phone under its label ("work", "home"), then the
   emails the same way. Press a row to copy its value.
3. Search: type a value that only an extra phone or email has (for example the second number). The contact is listed.
4. File > Export vCard 3.0 or 4.0 writes all the values back; Copy Contact (the card button) puts them in the text.
5. Delete the contact: its extra values go from `values.txt`.

## How to verify on Midget

Use `ssh midget` and the `bifrost pc midget` controls (see `docs/bifrost-pc-workflow.md`).

1. Publish and update Midget, then `bifrost pc midget open-app is.olibuijr.contacts.desktop`.
2. Put a `.vcf` with two phones (one `TYPE=work`, one `TYPE=home`) and two emails in the Contacts Import folder and
   choose File > Import vCards. The status line reads "Imported 1 contacts" with no "not kept" count.
3. Open the contact with `bifrost pc midget tap-window Contacts X Y` on its row. The card shows four or more value
   boxes: the phones and emails under their labels, each on its own box. Tap the second phone: the "Copied" toast appears.
4. Search for the second phone number in the search field: the contact stays listed; search for a number that is
   nowhere: the list is empty.
5. File > Export vCard 3.0: `Export/contacts-3.0.vcf` holds both `TEL` lines and both `EMAIL` lines.
6. Dark and Light (`bifrost pc midget appearance dark|light`): the list and card stay on the content colour.
7. Restart the app: the extra values are still on the card (they come from `values.txt`).

Host checks (not run here, since the lead builds once): `cargo test -p bifrost-contacts` in `springboard/`. The new tests
are `extras::tests` (file text, escapes, limits, search, prune), `exchange::tests` (extra values on import and export,
the six-value limit), `contacts::tests` (card rows, list search, delete) and `services::tests` (plain text with extras).
The compliance golden may need no change: the card and list look the same for contacts with one value each.

## Reference behaviour and evidence

- `CNContact` (`https://developer.apple.com/documentation/contacts/cncontact`): `phoneNumbers`, `emailAddresses` and
  `postalAddresses` are arrays of labelled values, so a contact holds several of each, and `note` is one string.
  Read with `python3 -I tools/parity/docc.py contacts/cncontact` and rendered with `tools/parity/rea-page.sh`.
  REA evidence `ev_b45cb8acf478edb76756b8542e953f729a5267a33395f11337f2733a93fbb6f4`.
- Labels follow the vCard `TYPE` values the app already maps (`cell` is Mobile; `other` has no type); the label
  sets are the store's (`contactstore::LABELS`).
- vCard import and export: the wire5 note and `docs/parity/wire2-contacts.md` hold the earlier evidence.

## Remaining gaps

- The contact editor still edits one phone, one email and one address. Adding or changing the extra values from the
  editor needs a dynamic list of fields and is not built; the extra values come from vCard import only. The matrix
  rows "Multi-value properties and labels" and "Labeled values and predefined labels" stay partial for this reason.
- Extra addresses: the format and the card and export handle them, but no import creates them yet (the vCard reader
  keeps one `ADR` per card, as before). They appear only if a values file already holds them.
- Duplicate merge: `dupes::groups` finds duplicate contacts (the Duplicates list), but there is no merge model in
  `springboard/ui/src/` to merge two contacts' values, so the app does not merge them. Merging needs a model first.
- Authorization and privacy (`contact_access`), contact containers and groups (`contact_groups`) and the contact picker
  are not changed in this round; the wire2 and wire3 notes hold their state.
- Store-level changes (the single-value store format in `springboard/src/shell/contactstore.rs`) are outside this app,
  so the store's own search (the Shell search panel) does not see the extra values. Only the Contacts app's list does.
- Undo of a contact's delete or edit does not restore its extra values (the in-app undo covers the store only).

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| contacts | Contact data model (CNContact, CNMutableContact) | springboard/apps/contacts-desktop/src/extras.rs (extra values beside the store); src/contacts.rs (`card_entry`, `card_rows`) |
| contacts | Labeled values and predefined labels | springboard/apps/contacts-desktop/src/extras.rs (`ValueKind`, `Extra::well_formed` against `contactstore::LABELS`); src/exchange.rs (`type_of`, label mapping) |
| contacts | vCard serialization | springboard/apps/contacts-desktop/src/exchange.rs (`export` writes extra values and addresses) |
| address-book | Multi-value properties and labels | springboard/apps/contacts-desktop/src/extras.rs; src/contacts.rs (card rows for extra values) |
| address-book | Search | springboard/apps/contacts-desktop/src/contacts.rs (`list_hits`, used by `rebuild`) |
| address-book | vCard import and export | springboard/apps/contacts-desktop/src/exchange.rs (`import` keeps extra values; `export` writes them) |
| address-book | Sharing and copying contact values | springboard/apps/contacts-desktop/src/services.rs (`plain_text`, `contact_item`, `copy_contact` with extra values) |
