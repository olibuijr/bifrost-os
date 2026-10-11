# Theme 30: Access authorization (contacts)

Three pure models in `springboard/ui/src` (bifrost-ui) close the authorization, picker-grant, partial-fetch and
postal-address gaps of the contact frameworks. Each model is std only, has no I/O, and has its own tests. No
SDK wiring yet: the shell and the Contacts app do not call these modules, so the matrix rows are `partial`.

## What was built

| Module | What it does | Official API it mirrors |
|---|---|---|
| `springboard/ui/src/contact_access.rs` | Per-app access state: `NotDetermined`, `Restricted`, `Denied`, `Authorized`, `Limited`. The prompt is answered once (`request`); Settings can change it (`change_in_settings`) unless the system restricts the app. A `Picked` choice grants only the chosen contact identifiers and is read-only. `can_read`, `check_write` and `visible` gate the store. | `CNAuthorizationStatus` (notDetermined, restricted, denied, authorized, limited); the legacy `ABAddressBookGetAuthorizationStatus` / `ABAddressBookRequestAccessWithCompletion` pair maps onto the same states. |
| `springboard/ui/src/contact_fetch.rs` | Key-based partial records. `Keys::new` needs at least one key; `project` returns a `Partial` that holds only the fetched fields. Reading an unfetched field is `Error::NotFetched`, not an empty value. | `CNContactFetchRequest` (`init(keysToFetch:)`), `keysToFetch`, and the `CNContactPropertyNotFetchedExceptionName` behaviour for unfetched keys. |
| `springboard/ui/src/postal_address.rs` | Structured postal address (street, sub-locality, city, sub-administrative area, state, postal code, country, ISO code). `validate` checks length, control characters and the two-letter country code. `lines`, `format` and `single_line` print the parts in a country layout (default, US, GB, IS). | `CNPostalAddress` (street, subLocality, city, subAdministrativeArea, state, postalCode, country, isoCountryCode) and `CNPostalAddressFormatter` (`string(from:)`). |

Official pages used (DocC via `tools/parity/docc.py`):
- https://developer.apple.com/documentation/contacts/cnauthorizationstatus
- https://developer.apple.com/documentation/contacts/cncontactfetchrequest
- https://developer.apple.com/documentation/contacts/cnpostaladdress
- https://developer.apple.com/documentation/contacts/cnpostaladdressformatter

REA evidence (rendered page, `tools/parity/rea-page.sh`):
- `ev_e7f06967c24c072f47716874c19d6cda6fcf5bde8d47897214e0f738f6f2368a` (https://developer.apple.com/documentation/contacts)

## Limits (honest)

- `contact_access`: a limited grant is modelled, but the picker UI is not built. Nothing enforces the gate yet: the
  store still reads the whole file, so the status is advisory until the shell calls `can_read` before returning a record.
- `contact_fetch`: the model projects one record. The store does not yet take a `Keys` argument, and there is no
  predicate, unified-contact or mutable-object option.
- `postal_address`: four country layouts only (default, US, GB, IS). No locale-aware name order and no localized key
  names. The sub-administrative area is stored but not printed.
- Groups, change history and vCard were already modelled by wave 1 (`contact_groups.rs`, `contact_changes.rs`,
  `vcard.rs`). They are not claimed here.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| address-book | Access authorization | partial | springboard/ui/src/contact_access.rs |
| address-book | Formatted postal addresses | partial | springboard/ui/src/postal_address.rs |
| contacts | Authorization and privacy | partial | springboard/ui/src/contact_access.rs |
| contacts | Contact picker and access UI (ContactsUI) | partial (model only; no picker UI) | springboard/ui/src/contact_access.rs |
| contacts | Fetch with key-based property selection | partial | springboard/ui/src/contact_fetch.rs |

## Verification

No builds were run, by rule. The tests are in each module (`mod tests`): 8 in `contact_access.rs`, 6 in
`contact_fetch.rs`, 7 in `postal_address.rs`. `python3 tools/check-names.py` exits 0.
`tools/parity/parity.py search` failed on a corrupt embedding cache (`keys.npy` bad CRC in
`tools/parity/semantic.py`), so the existing-module check used the index and the file listing instead.
