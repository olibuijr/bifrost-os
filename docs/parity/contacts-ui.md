# Contacts UI parity (wave 2)

Two pure Rust models in `springboard/ui/src` (bifrost-ui), with no drawing, no shell IPC and no new crates. They
cover the contact viewer's card rows and the contact picker's list, search and returned choice. The SDK views that
would let other apps embed them are not built yet, so the framework stays `partial`.

## What was built

- `springboard/ui/src/contact_card.rs`: the shared types (`Key`, `Field`, `Entry`) and the card model for a contact
  viewer. A `Card` has a kind (`Existing`, `Unknown`, `New`), a title fallback (name, then alternate name, then a
  placeholder for the kind), displayed keys, an editing flag and an actions flag. `Card::rows` returns the rows in
  card order, skipping empty values. Phone rows offer `Action::Call` and email rows `Action::Email` when actions are
  allowed. `highlight_row` finds a row by key and optional label.
- `springboard/ui/src/contact_picker.rs`: the picker model. `Mode::Contact { need }` lets the user choose a whole
  contact, and `need` limits choices to contacts with a value under one key. `Mode::Property { keys }` lets the user
  open a contact and choose one value. `Picker::visible` sorts by name or ranks matches for a query (through
  `fuzzy::rank`). `select`, `choose` and `back` return a `Choice` only for the final selection, so the requesting
  app never gets the store. Contacts disabled with `disable` stay listed but cannot be chosen.

Both are registered in `springboard/ui/src/lib.rs` with one `pub mod` line each. Tests: 6 in `contact_card.rs`,
7 in `contact_picker.rs`. Nothing was built or run, per the task rules.

## Official API names mirrored

| Official API | Where it is mirrored |
|---|---|
| `CNContactViewController` (contact viewer: new, unknown, existing), `displayedPropertyKeys`, `allowsEditing`, `allowsActions`, `alternateName` | `contact_card.rs` `Card` (kind, displayed, allows_editing, allows_actions, alternate_name) |
| `CNContactPickerViewController`, `displayedPropertyKeys`, `predicateForEnablingContact`, `predicateForSelectionOfContact`, `predicateForSelectionOfProperty` | `contact_picker.rs` `Picker` (`with_displayed`, `disable`, `Mode::Contact { need }`, `Mode::Property { keys }`) |
| `CNContactPicker` (popover picker) | Same model as the picker above; no popover is drawn |
| `highlightProperty(withKey:identifier:)` | `contact_card.rs` `highlight_row` |

Not mirrored: `ContactAccessButton` and the limited-access permission model (matrix row "Contact access button", n/a);
the contact store and search through the SDK (needs shell IPC); the multi-select picker mode; linked contacts,
parent group and container.

## REA evidence

- Framework landing page, Contacts UI: `ev_f980ba1055c2e435049b920d477b13fd19c67e36c4e33ceab4efaf27d9dbe13e`
  (https://developer.apple.com/documentation/contactsui)
- Picker view controller: `ev_b06425e4043f0784556684d4ef70afd3aed1365ed700ba027d514d7fb14e9a81`
  (its page says the picker gives the app only the final selection, with no access grant, and that predicates take
  effect only before presentation; both match the model's design)
- Viewer view controller: `ev_60b29c3553ca08f502b7b6cc575267f42c62809ab2531310d1dd2c6dc2ddce77`

## Matrix rows closed

Status values follow `docs/parity-matrix/agent-brief.md`. The matrix itself is not edited here.

| Framework slug | Feature (as written in `docs/parity-matrix/data/contacts-ui.json`) | New status | BifrOSt path |
|---|---|---|---|
| contacts-ui | Contact picker (popover and modal) | partial | `springboard/ui/src/contact_picker.rs` (model only; no SDK picker or popover yet) |
| contacts-ui | Contact viewer | partial | `springboard/ui/src/contact_card.rs` (card model; the app keeps its own drawing) |

## Gaps left

- No SDK entry point for the picker: the shell must draw it and return the choice to the app over IPC.
- No SDK card view: the card rows still come from the Contacts app's own drawing.
- Multi-select picker mode and linked contacts are not modelled.
