# Wire 3: Contacts (contact card, vCard, name order check)

Release 1.5.0 of the Contacts app (`springboard/apps/contacts-desktop`, package `bifrost-contacts`). This round wires
the person view (`contact_card`) into the contact detail pane and moves the list and the detail pane onto the content
colour. The vCard import and export that the wire2 note listed as open was already wired in an earlier commit
(`967e6066`, `src/exchange.rs`), so this round only confirms it. The name display order is still not wired (see gaps).

## What is now wired

| Model | What the user can do | Where in the app |
|---|---|---|
| `contact_card` (`Card`, `Entry`, `Field`, `Key`, `Kind`, `Row`) | The contact card lists its phone, email and address through the model: each row has the label the contact chose (Mobile, Home, Work, Other), empty values have no row, and the rows keep the model's order. Copy works from the same rows. | Contact detail pane (open a contact in the list). `card_entry`, `card_rows` and `value_slot` in `src/contacts.rs`. |
| `vcard` and `exchange.rs` (already wired before this round) | Import vCard 3.0 or 4.0 files from the Contacts Import folder; export all contacts as vCard 3.0 or 4.0 to the Contacts Export folder. | Toolbar import and export items (`BAR_IMPORT`, `BAR_EXPORT`, `BAR_EXPORT_V4`). |

Other changes:

- `src/contacts.rs`: the detail pane, the list column and the toolbar band fill with `CONTENT_BG` instead of
  `DETAIL_BG`. `DETAIL_BG` is pure black in Dark, so the window now uses the dark grey of `CONTENT_BG`, matching other
  apps. The unified toolbar band uses the same colour.
- The card's value rows have the same hit positions as before. A tap on a row still copies that value (`Target::Copy`
  with the slot index 0 phone, 1 email, 2 address).
- `Cargo.toml`, `manifest.toml`: version 1.5.0. `CHANGELOG.md`: 1.5.0 entry.
- Tests: `the_card_shows_the_values_a_contact_has_under_their_labels` (rows, labels, and slots from the layout).

## How a user reaches it

1. Open Contacts and pick a contact in the list (or All Contacts). The card shows the name, company, then one
   box per value the contact has: the label on the left, the value in blue.
2. A contact with a phone and an address but no email shows two value boxes, not three.
3. Press a value box: the value is copied and the toast "Copied" shows, as before.
4. Settings (gear) and the sort order still work as in wire2.

## How to verify on Midget

Use `ssh midget` and the `bifrost pc midget` controls (see `docs/bifrost-pc-workflow.md`).

1. Publish and update Midget, then `bifrost pc midget open-app is.olibuijr.contacts.desktop`.
2. Make or pick a contact with a phone (labelled Work) and an address, and no email. Open it with
   `bifrost pc midget tap-window Contacts X Y` on its row. Screenshot: two value boxes, the first labelled "work",
   the second "home" for the address, with no empty email box.
3. Tap the phone box: the "Copied" toast appears.
4. Check the whole window in Dark (`bifrost pc midget appearance dark`): the list and the card sit on dark grey, not
   black, and the toolbar band matches. Then Light: the content is white.

Host checks (not run here, since the lead builds once): `cargo test -p bifrost-contacts` in `springboard/`. The compliance
gate (`tests/compliance.rs`) and its snapshots may need re-blessing (`BIFROST_BLESS=1`) because the detail pane's
background changes. Cargo.lock lists `bifrost-contacts` 1.4.0; a build updates it to 1.5.0.

## Reference behaviour and evidence

- `CNContactViewController` (contact viewer): `https://developer.apple.com/documentation/contactsui/cncontactviewcontroller`,
  read with `tools/parity/docc.py` and rendered with `tools/parity/rea-page.sh`. REA evidence
  `ev_ec4a3b9422e3679128a5e0c2088bfce3a5556e46f4699d6a6ad2cb8801a4ddfd`. The viewer shows one contact with an optional
  alternate name and message, a set of displayed property keys, and `allowsEditing` and `allowsActions` switches. The
  card model follows the displayed keys and the actions; the app keeps its own editor and does not use the
  alternate name, message or linked contacts yet.
- Earlier evidence for groups, the picker and sort order is in `docs/parity/wire2-contacts.md`.

## Remaining gaps

- Name display order (first name or last name first): `contact_display` has only the sort order (`SortOrder`). It has
  no name display order, so no Settings row can save one. This needs a model change in `springboard/ui/src/` (outside this
  app). The matrix row "Predefined display preferences" stays partial.
- Card actions: `Action::Call` and `Action::Email` on the model's rows are not used, because the app has no call or mail
  hand-off. Taps copy the value, as before.
- Card `alternate_name`, `message` and linked contacts (`shouldShowLinkedContacts`) are not shown. The card title is the
  contact's display name, as before.
- The contact viewer is only inside Contacts. No SDK call lets another app show a contact card (matrix "Contact viewer"
  note).
- Cargo.lock and the snapshot golden are left for the lead's build.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| contacts-ui | Contact viewer | springboard/ui/src/contact_card.rs (wired: `apps/contacts-desktop/src/contacts.rs`, `card_rows`, `card_entry`, `value_slot`) |
| contacts | vCard serialization | springboard/ui/src/vcard.rs (wired: `apps/contacts-desktop/src/exchange.rs`, earlier commit 967e6066) |
| contacts | Predefined display preferences | springboard/ui/src/contact_display.rs (sort order wired; name display order still missing) |
