# Wire 2: Contacts (groups, people picker, display settings)

Release 1.4.0 of the Contacts app (`springboard/apps/contacts-desktop`, package `bifrost-contacts`). It wires three
pure models from `springboard/ui/src` into the app so a user can reach them. The models themselves are unchanged.

## What is now wired

| Model | What the user can do | Where in the app |
|---|---|---|
| `contact_groups` (`Groups`) | Make a group, add or remove a contact (existing), rename a group, delete a group (contacts stay), add many people at once | Sidebar Groups section; toolbar with a group shown |
| `contact_picker` (`Picker`, `Mode::Contact`, `Choice`) | Add People: every contact is listed, a press adds that person to the shown group, members cannot be chosen twice | Toolbar Add People (group shown), then click rows, Done |
| `contact_display` (`ContactsDefaults`, `SortOrder`) | Sort Order row: System Default, Family Name, Given Name, Keep Order; saved in `display.txt` | Toolbar Settings (gear), Sort Order row, Done |

Files changed (all inside the app folder, plus this note):

- `src/contacts.rs`: group rename, delete and picker actions; the Settings screen; list sorting by the chosen order.
- `src/group_file.rs`: groups file format 2 (`contact-groups 2`) stores each group number. A deleted group leaves a gap,
  so the numbers must be written. Format 1 files still load in order.
- `src/display_file.rs` (new): `display.txt` (`contacts-display 1`, one `sort` line).
- `src/main.rs`: `mod display_file;`.
- `manifest.toml`, `Cargo.toml`: version 1.4.0. `CHANGELOG.md`: 1.4.0 entry.

The sidebar keeps its own list of group identifiers (`group_ids`), because `Groups` has no way to list identifiers.
`Groups::remove` returns the removed identifiers, which the app uses to update that list.

## How a user reaches it

1. New Group (toolbar) makes "Group n" and shows it.
2. With a group shown, the toolbar gains Add People, Rename Group and Delete Group.
3. Rename Group moves the name into the search box's place. Return saves, Escape or any other control cancels, and
   an empty name keeps the old one (error line).
4. Add People shows every contact. Click a contact to add it; a contact already in the group reports "already in the
   group". Done returns to the group.
5. Settings (gear, far right of the list toolbar) opens the Settings screen. The Sort Order row cycles the choice. Done
   returns to the list. An open editor blocks Settings so unsaved edits are not lost.

## How to verify on Midget

Use `ssh midget` for the lab machine and the `bifrost pc midget` controls (see `docs/bifrost-pc-workflow.md`).

1. Publish and update Midget as usual, then open Contacts (`bifrost pc midget open-app is.olibuijr.contacts.desktop`).
2. New Group, then screenshot: the sidebar shows "Group 1" and the toolbar shows Add People, Rename Group, Delete Group.
3. Rename Group, type "Friends", Return. The sidebar row reads "Friends". Restart the app: the name stays
   (`groups.txt` is `contact-groups 2`).
4. Add People, click two contacts with `bifrost pc midget tap-window Contacts X Y` (window-local px), click one of them
   again: the status line says it is already in the group. Done. The group shows the two contacts.
5. Delete Group: the group leaves the sidebar, the contacts stay in All Contacts.
6. Settings, press the Sort Order row: it changes Family Name, Given Name, Keep Order, System Default in turn. Check
   that the list order changes at each step. Restart: the last choice stays (`display.txt`).
7. Check the Light and Dark appearances (`bifrost pc midget appearance light|dark`).

Host checks (not run for this note, since the lead builds once): `cargo test -p bifrost-contacts` in `springboard/`
covers the file formats, the picker, the sort comparator and the Settings cycle. The compliance gate
(`tests/compliance.rs`) needs its golden re-blessed (`BIFROST_BLESS=1`), because the list toolbar gains a Settings
button. Cargo.lock lists `bifrost-contacts` 1.3.0; a build updates it to 1.4.0.

## Reference behaviour and evidence

Official pages read with `tools/parity/docc.py` and rendered with `tools/parity/rea-page.sh`:

- `CNGroup` (groups): `https://developer.apple.com/documentation/contacts/cngroup`. REA evidence
  `ev_f19da95c1d6bbe11137d640cd773804828d06800aed78f3a1e2fe6d079a22b70`. Groups have a name and an identifier, and
  subgroups are found through predicates. The app uses flat groups; the model's subgroup support is not shown yet.
- `CNContactPickerViewController` (picker): `https://developer.apple.com/documentation/contactsui/cncontactpickerviewcontroller`.
  REA evidence `ev_d17155f6a72584fb591d0e94a828d2325a54845ca33d721093282f2734ea737c`. The picker "has access only to
  the user's final selection" and has an enabling predicate, which matches `Picker::disable` and `Picker::select`.
- `CNContactSortOrder` (sort order): `https://developer.apple.com/documentation/contacts/cncontactsortorder`. REA evidence
  `ev_398f0cbfa80f60e726c78d52de134435c412820a53aea85ce76047965ddae0a0`. Options are none, userDefault, givenName and
  familyName; the app offers the same four.

## Remaining gaps

- Person view (`contact_card`, `Card`, `Entry`): not wired. The contact card still uses the app's own layout, so the
  card's displayed-keys and row actions are not used.
- Name display order (first-last or last-first): `contact_display` has no such setting, and the model does not supply
  one. The app only has the sort order. This needs a model change outside this wave.
- Default country code (`ContactsDefaults::set_country_code`): stored by the model but no phone formatting in the app
  uses it, so the Settings screen does not offer it.
- The people picker is only inside Contacts. Other apps still cannot ask the shell to pick a contact (no SDK channel;
  see `contact_access` in the matrix).
- Groups are flat. Subgroups (`Groups::add(name, Some(parent))`, `move_to`) are not offered.
- Snapshot golden and Cargo.lock updates are left for the lead's build (see above).

## Matrix rows (`docs/parity-matrix/data/contacts.json`)

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| contacts | Containers and groups | springboard/ui/src/contact_groups.rs (wired: `apps/contacts-desktop/src/contacts.rs`, `group_file.rs`) |
| contacts | Contact picker and access UI (ContactsUI) | springboard/ui/src/contact_picker.rs (wired: Add People in `apps/contacts-desktop/src/contacts.rs`) |
| contacts | Formatting names and addresses | springboard/ui/src/contact_display.rs (sort order wired: `apps/contacts-desktop/src/contacts.rs`, `display_file.rs`; name order still missing) |
