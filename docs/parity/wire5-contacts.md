# Wire 5: Contacts (menus, vCard from the File menu, name order)

Release 1.6.0 of the Contacts app (`springboard/apps/contacts-desktop`, package `bifrost-contacts`). This round wires
the menu-bar side of the vCard exchange, a View menu for the sidebar and the name order, and the name order itself. The
name display order was the open gap in the wire3 note. The app's own models are unchanged: no file under
`springboard/ui/` was touched, so the name order lives in the app (`names.rs`) and the saved settings in
`display_file.rs`.

## What is now wired

| Feature | What the user can do | Where in the app |
|---|---|---|
| vCard import and export (menu) | File > Import vCards... reads every `.vcf` in the Contacts Import folder, as the toolbar Import button does. File > Export vCard 3.0... and Export vCard 4.0... write the Contacts Export folder. | `main_menu()` and `menu_action()` in `src/contacts.rs`; `main.rs` forwards them to the shell. Same functions as the toolbar (`import_vcards`, `export_vcards`). |
| Name display order | View > First Name First (default) or Last Name First. Last name first shows "Lovelace, Ada" in the list rows and in the card title. The list order is still the Settings sort order. The choice is saved in `display.txt`. | `names::NameOrder`, `names::shown()` in `src/names.rs`; `set_name_order()`, `save_display()` in `src/contacts.rs`; the `name` line in `src/display_file.rs`. |
| Sidebar from the menu | View > Hide Sidebar / Show Sidebar, the same as the toolbar button. | `MENU_SIDEBAR` calls `bar(BAR_SIDEBAR)`. |
| Labelled values in the editor | Already wired in earlier rounds: the phone, email and address fields in the editor cycle through their labels (Mobile, Home, Work, Other) with the label button (`labels_idx`), and the card shows the chosen label. | `src/contacts.rs` (`labels_idx`, `card_entry`). Unchanged this round. |

The app's main menu merges into the shell's File and View menus (`desktop_bar.rs` `shell_menu`), so the bar keeps the
order App, File, Edit, View, Window, Help. The compliance gate's menu rules hold: titles unique, every item has a title
and a non-zero id.

Other changes:

- `Cargo.toml`, `manifest.toml`: version 1.6.0. `CHANGELOG.md`: 1.6.0 entry at the top.
- Tests: `names::tests::the_name_order_decides_which_part_leads`; `display_file` tests (`a_missing_file_is_the_defaults_and_a_choice_round_trips`,
  `a_file_from_before_the_name_order_shows_first_names_first`, `a_foreign_or_unknown_file_is_refused` with an unknown
  name value); `contacts` test `the_file_menu_and_view_menu_are_the_app_menus_and_name_order_is_saved`.
- The existing test `the_settings_row_cycles_the_sort_and_saves_it` now reads the new `(defaults, name order)` pair.

## How a user reaches it

1. File > Import vCards... imports the `.vcf` files in the Contacts Import folder. The status line says how many were
   added. File > Export vCard 3.0... or 4.0... writes `contacts-3.0.vcf` or `contacts-4.0.vcf` to the Export folder.
   Both are greyed out when the contacts file could not be read.
2. View > Last Name First: a contact "Ada Lovelace" shows as "Lovelace, Ada" in the list and as the card title. The
   check mark moves to Last Name First. Choose First Name First to go back.
3. View > Hide Sidebar hides the lists sidebar (View > Show Sidebar brings it back).

## How to verify on Midget

Use `ssh midget` and the `bifrost pc midget` controls (see `docs/bifrost-pc-workflow.md`).

1. Publish and update Midget, then `bifrost pc midget open-app is.olibuijr.contacts.desktop`.
2. Open the File menu on the menu bar and choose Import vCards... with a `.vcf` in the Contacts Import folder. The
   status line names the count. Then File > Export vCard 3.0...: the Export folder holds `contacts-3.0.vcf`.
3. Open a contact with a first and last name. Open View and choose Last Name First: the list row and the card title read
   "Last, First". Reopen View: the check mark is on Last Name First. Restart the app: the order holds.
4. View > Hide Sidebar: the sidebar goes; the menu reads Show Sidebar.
5. Dark and Light (`bifrost pc midget appearance dark|light`): the content stays on the content colour.

Host checks (not run here, since the lead builds once): `cargo test -p bifrost-contacts` in `springboard/`. The snapshot
golden may change only if the list or card title text changes for a contact with a name; the default order is unchanged,
so the golden should hold.

## Reference behaviour and evidence

- `CNContactFormatter` (`https://developer.apple.com/documentation/contacts/cncontactformatter`): formats a contact's name
  with a display name order (`displayNameOrder`) and a name delimiter. Read with `tools/parity/docc.py contacts/cncontactformatter`
  and rendered with `tools/parity/rea-page.sh`. REA evidence `ev_76d51d0d772e1ada18a782679a4df5094f2b12ea1f2ce23d226fa5d58bb24ac6`.
  The name order menu follows the display name order: given name first, or family name first.
- vCard: `CNContactVCardSerialization` and the `ABPersonCreateWithVCardRepresentation` family, as in the wire2 and wire3
  notes. The File menu uses the existing `exchange.rs` (3.0 and 4.0).

## Remaining gaps

- Multi-value properties: a contact still has one phone, one email and one address (the labels are per value). Several
  phone numbers or emails per contact need a store format change (`springboard/src/shell/contactstore.rs`), outside this
  app. The matrix row "Multi-value properties and labels" stays partial.
- Name order is an app setting, not a model: `springboard/ui/src/contact_display.rs` (`ContactsDefaults`) has no name order,
  so the rule lives in `names.rs`. Moving it into the model needs a change in `springboard/ui/` (outside this app).
  Last-name order uses ", " between the names; the locale's name delimiter is not read.
- Contact fetch with predicates (`CNContactFetchRequest` with predicates): the list filters are the sidebar lists, groups
  and Recently Changed; no predicate search is exposed.
- Change history (`CNChangeHistoryFetchRequest`) and the contacts-changed notification: no change log is read by other apps.
  The in-app Recently Changed list (`history.rs`) is session-only.
- Card `alternate_name`, `message` and linked contacts stay unshown, as in wire3.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| contacts | Formatting names and addresses | springboard/apps/contacts-desktop/src/names.rs (`NameOrder`, `shown`); view menu in src/contacts.rs (`main_menu`, `set_name_order`) |
| contacts | Predefined display preferences | springboard/apps/contacts-desktop/src/display_file.rs (name line); springboard/ui/src/contact_display.rs (sort order) |
| contacts | vCard serialization | springboard/apps/contacts-desktop/src/exchange.rs (File menu: src/contacts.rs `menu_action`) |
| address-book | vCard import and export | springboard/apps/contacts-desktop/src/exchange.rs (File menu Import vCards..., Export vCard 3.0... and 4.0...) |
| address-book | Multi-value properties and labels | springboard/apps/contacts-desktop/src/contacts.rs (labelled single values: `labels_idx`, `card_entry`); multi-value stays partial |
