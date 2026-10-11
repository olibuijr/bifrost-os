# Contacts parity: what was added

Framework status stays `partial` (see `docs/parity-matrix/data/contacts.json`). This round adds three pure models to
`springboard/ui/src` (bifrost-ui, std only, no unsafe). None of them is wired to the shell contact store, the Contacts
app or the SDK yet, so apps still have no contacts API.

## Built

| Module | Mirrors | What it does | Tests |
|---|---|---|---|
| `springboard/ui/src/vcard.rs` | `CNContactVCardSerialization` (`contacts(with:)`, `data(with:)`) | Reads vCard 3.0 text into `Card` values (names, organization, labelled phones and emails, note). Unfolds continuation lines and unescapes values; `Card::to_vcard` escapes and folds lines to 75 bytes, CRLF endings. Skips empty values and unknown properties. | 7 |
| `springboard/ui/src/contact_groups.rs` | `CNGroup`, `CNMutableGroup`, `addMember`, `removeMember`, `addSubgroup`, `removeSubgroup` | A tree of named groups holding contact identifiers. Recursive membership, moves that refuse cycles (`move_to`), subtree removal, memberships dropped when a contact is deleted. | 7 |
| `springboard/ui/src/contact_changes.rs` | `CNSaveRequest` (add, update, delete), `CNChangeHistoryFetchRequest` (`startingToken`), `CNChangeHistoryEvent` family | `Store::save` applies a batch all or nothing and validates each record; every applied change gets a sequence number. `Store::changes_since(token)` returns later events, or `Expired` when the bounded history no longer reaches the token. | 8 |

Groups use `move_to` for the subgroup relation, so `addSubgroup` and `removeSubgroup` map to `move_to(child, Some(parent))`
and `move_to(child, None)`.

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`.

## Official API this follows

Apple documentation (fetched with `tools/parity/docc.py`, rendered page captured with REA):

- Framework overview: https://developer.apple.com/documentation/contacts
- `CNSaveRequest`: https://developer.apple.com/documentation/contacts/cnsaverequest
- `CNChangeHistoryFetchRequest`: https://developer.apple.com/documentation/contacts/cnchangehistoryfetchrequest
- `CNContactVCardSerialization`: https://developer.apple.com/documentation/contacts/cncontactvcardserialization
- `CNGroup`: https://developer.apple.com/documentation/contacts/cngroup

## REA evidence

- `ev_98fb31345b037e83a1ae60a63cdc50da91e06c47360142d0e3146310a7e6d94f`: rendered Contacts framework overview
  (title "Contacts | Apple Developer Documentation").

## Not built, and why

- No store wiring: `contactstore.rs` still rewrites its flat file on every save and keeps single-valued fields. Moving it
  to the models changes the on-disk format and needs its own round.
- No SDK API, no permission model, no contacts-changed notification to apps.
- No containers (accounts), and no group or history persistence.
- vCard addresses, dates, photos and custom properties are not read or written. Share still uses its own vCard writer
  in `springboard/src/shell/share/mod.rs`.
- No contact picker, no key-based partial fetch, no display preferences.

## Verification

Per the round's rules, nothing was built or run. `python3 tools/check-names.py` reports no hits (exit 0).
The new unit tests (22 in total) have not been compiled or executed; the build stage runs them.
