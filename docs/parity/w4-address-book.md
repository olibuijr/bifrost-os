# Address Book (parity wave 4)

Official reference: https://developer.apple.com/documentation/addressbook (the page says the framework is deprecated
from macOS 10.11 and points to the Contacts framework).

REA evidence of the rendered pages:

- Framework landing page: `ev_06a6e7e931b56aba3909eb9bd1ff4bd630c7c0f03d6ee2da6a0041cf8c7c8607`
- `ABExternalChangeCallback` page (https://developer.apple.com/documentation/addressbook/abexternalchangecallback):
  `ev_9187d5ce4d58841cc104f5d3b16ca26692a5d5ed6b88c60b49ffd8a3ea8c14b9`

Wave 1 and 2 already added the group tree, the picker and card models, the change history store and the postal and
search models. This round closes the two gap rows those did not cover. Both are pure models in `bifrost-ui`
(`springboard/ui/src`). Neither is wired into the contact store or an SDK call yet, so the matrix status is `partial`.

## What was built

### `springboard/ui/src/contact_photo.rs` (`Photos`, `format_of`, `fit_size`)

The picture of each contact, kept beside its record, mirroring the image methods of an address book person.

- `Photos::set(contact, data)` stores a picture and replaces the old one. It accepts PNG and JPEG only, detected from
  the file signature. Empty data, data over `MAX_BYTES` (8 MiB) and unknown formats are refused, and the old picture
  stays.
- `data`, `format`, `remove` (clears the picture, `false` when there was none), `len`, `is_empty`.
- `fit_size(width, height, max)` works out a thumbnail that keeps the aspect ratio inside a square, with integer math.
  A picture that already fits keeps its size, and a non-zero side never drops below 1 pixel.

Tests: 6 (signature detection, store and read back, replacement, refused pictures keep the old one, removal, thumbnail
sizes).

### `springboard/ui/src/contact_changes.rs` (`Observers`)

External-change observers, mirroring the external change callback of the address book. A client registers with the
token it has seen. After a save the caller passes the store's token to `notify`, which returns the observers that are
behind it and moves each one to that token. A save with several records therefore tells each observer once.

- `register(token)` returns an identifier that is never reused, `unregister(id)` returns whether it existed.
- `notify(token)` returns the due identifiers in identifier order. Each due observer is moved to `token`.
- `len`, `is_empty`.

Tests: 4 added to the existing `contact_changes` test module (registration, unregistration, one notice per store move
with a real `Store`, and a fetch of the change an observer was told about).

## Official API names mirrored

- `ABPerson`, `setImageData(_:)` and `imageData()` (Managing Images section of the `ABPerson` page), for `Photos`.
- `ABExternalChangeCallback`, "invoked on an address book when the Address Book database is modified by another
  address book instance", for `Observers`.
- `ABAddressBookRegisterExternalChangeCallback` and `ABAddressBookUnregisterExternalChangeCallback` are named in the
  matrix row but were not found in the DocC index, so the register and unregister names here are a mirror of the
  matrix, not a checked signature.

## Matrix rows

| Framework | Feature (as written in data/address-book.json) | New status | BifrOSt path |
|---|---|---|---|
| Address Book | Contact photos | partial | springboard/ui/src/contact_photo.rs (Photos::set, data, format, remove) |
| Address Book | Change notifications | partial | springboard/ui/src/contact_changes.rs (Observers::register, unregister, notify) |

Rows checked and found already partly covered by earlier waves (not edited here, the lead updates the matrix):

- Address Book | Groups: `springboard/ui/src/contact_groups.rs` (group tree and membership) from wave 1.
- Address Book | People picker and person view: `springboard/ui/src/contact_picker.rs` and `contact_card.rs` from wave 1.

## Not done in this round

- The contact store, the Contacts app and the SDK do not use `Photos` or `Observers` yet. The shell still keeps one
  record per contact with no picture field.
- The shell does not call the observers after a save from another process, and the callbacks are not run here.
- The `os` row (the store has no app-facing SDK call) is unchanged.
