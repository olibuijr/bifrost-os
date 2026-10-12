# Wire: Contacts app (contacts-desktop 1.3.0)

Status: wired in the app, not built or run. This round ran no cargo, rustc or tests (lead builds once). Every new
module carries host unit tests; they are unexecuted until the lead's build.

## What is wired

| Feature | Matrix slug, feature (docs/parity-matrix/data) | Where in the app | How a user reaches it |
|---|---|---|---|
| vCard 3.0 and 4.0 import | `address-book`, "vCard import and export" | `src/exchange.rs` over `bifrost_app::controls::vcard` | Toolbar Import: reads every `.vcf` in `Contacts/Import` |
| vCard 3.0 and 4.0 export | `contacts`, "vCard serialization" | `src/exchange.rs` | Toolbar Export 3.0 / Export 4.0: writes `Contacts/Export/contacts-3.0.vcf` or `contacts-4.0.vcf` |
| Contact groups (flat) | `contacts`, "Containers and groups"; `address-book`, "Groups" (gap before) | `src/group_file.rs` over `bifrost_app::controls::contact_groups::Groups`; sidebar Groups section | Toolbar New Group, then open a contact and press Add to Group (Remove from Group when already a member) |
| Change history (this session) | `contacts`, "Change history and contacts-changed notification" | `src/history.rs` over `bifrost_app::controls::contact_changes::Store` | Sidebar Recently Changed |
| Name order and sort keys | `contacts`, "Formatting names and addresses" | `src/names.rs` (sort, section letter, fold, split) | The list sorts by family name then given name; sections are family-name initials |
| Duplicate detection | none (no matrix row; app-level, see gaps) | `src/dupes.rs` | Sidebar Possible Duplicates; Import skips a card that matches an existing contact |

The app keeps its own single-phone, single-email contact record (`springboard/src/shell/contactstore.rs`). The models
are adapted to it, not the other way round.

### Details

- **Import** reads 3.0 and 4.0 text. The adapter drops `KIND:group` cards (counted), strips `tel:` and `mailto:` URIs,
  drops `VALUE=uri`, and unquotes `TYPE="..."`. It keeps the first phone and first email (the rest are counted as
  "not kept"), the postal address (joined onto one line), the organization, and the note. A card with no name and no
  company, or a value longer than the store allows, is counted as invalid. Identifiers are assigned with
  `store::new_id`.
- **Export** writes one card per contact. The address is written as an `ADR` line (the model does not write one).
  4.0 output is the same logical lines with `VERSION:4.0` and quoted `TYPE` values, folded to 75 bytes.
- **Groups** are numbered from 1 in creation order, which is how the model hands out identifiers. The file format is
  described in `src/group_file.rs`. The app never removes a group, so the numbers stay contiguous. Writes go to a
  temporary file, then replace `groups.txt`. A groups file that cannot be read turns the group commands off so it is
  not overwritten.
- **History** keeps a map between the app's contact identifiers and the store's (the store numbers its own records, and
  `new_id` reuses identifiers). Contacts loaded at launch are not changes. A deleted contact drops out of the list.
- **Sort and sections**: `names::compare` orders by family-name key, then given-name key, then the full name, with the
  contact identifier as the last tie-break. Accents fold to their base letter; ð sorts after d and þ after z, ö and æ
  after z, the Icelandic order used in this repo's own choice (not taken from Apple's documentation).
- **Duplicates** match on: the same email (ignoring case), the same phone (last seven digits), or the same name (accents
  and case ignored) with no conflicting phone or email. Groups are built with union-find; name matches are compared only
  inside a bucket of equal names.

## How to verify on the test PC (Midget)

Build and publish go through the lead. Then, on Midget (`bifrost pc midget ...`):

1. Open Contacts (`bifrost pc midget open-app is.olibuijr.contacts.desktop`), then `bifrost pc midget state`.
2. Sort and sections: the list is ordered by family name; the section headers are family-name initials.
3. Import: copy a 3.0 file and a 4.0 file (one with a `KIND:group` card) into the Contacts `Import` folder beside
   `contacts.tsv`. Press Import. The status line reads "Imported N contacts, ...". Re-import the same files: every
   card is skipped as a duplicate.
4. Export: press Export 3.0 and Export 4.0. Check `Export/contacts-4.0.vcf` starts with `VERSION:4.0` and has
   `TEL;TYPE="..."` lines.
5. Groups: press New Group ("Group 1"), open a contact, press Add to Group, then check the sidebar row shows the
   member. Check `groups.txt` lists `group` and `member` lines.
6. Recently Changed: edit a contact; the sidebar row lists it. Possible Duplicates: lists contacts with a shared email or
   phone.
7. Tap with `bifrost pc midget tap-window APP X Y` (window-local pixels), never raw `tap`.

Compliance: the toolbar gained five buttons and the sidebar gained rows. The compliance snapshot golden
(`tests/snapshots/`) will probably need `BIFROST_BLESS=1` after a host run; this round did not regenerate it.

## REA evidence (official documentation, rendered)

- `ev_cbc8fe2755db5f766570657b31bd71e4ec3e5c5e9f3b2b261b2f517112919755`: CNContactVCardSerialization
  (`contacts(with:)`, `data(with:)`).
- `ev_a19f47caff04ed3eef6410ab1acf23b4aa7c9891cdb7a5ef47e8a552a8f0cd12`: CNGroup ("Contacts may be members of one or
  more groups").
- `ev_ed2bcbb4255e59de8cbed8a73b8ff09229dc5daf205cafe3444328e10b04d21f`: CNContactFormatter (name order and delimiter).
- `ev_5b87e65660593fb6e37fda14cc53a0e1416149078817090686ee5030c7d7c613`: CNContactSortOrder (`familyName`, `givenName`,
  `userDefault`). The docs do not say what the default is; the list uses family name explicitly.
- `ev_db4545875e619f4e9a88058c46df6109eb2cba12cb8e72417a00164f22a21df5`: CNChangeHistoryFetchRequest.

Official text from `tools/parity/docc.py` was read for the same five symbols. The docs do not describe a duplicate
finder; the duplicate rule is ours.

## Remaining gaps

- Multi-value contacts: the store holds one phone, one email, one address. Extra vCard values are counted and dropped
  on import; export writes what the contact holds.
- No contact photo, birthday, URL, relation or social profile in vCard files.
- Groups are flat. Subgroups (`move_to`), renaming and deleting a group have no UI. Groups are not written to vCard
  files (3.0 `CATEGORIES`, 4.0 group cards).
- Change history lasts for one run and is not saved; there is no contacts-changed notification to other apps.
- Duplicate detection is app code, not a model in `springboard/ui`. Merging duplicates is not offered.
- Import and export use fixed folders beside the contacts file; there is no file picker.
- Name formatting is the list order and section letter only. No name-order preference, no postal address formatter
  (`CNPostalAddressFormatter`) and no attributed name.
- Sidebar symbols (`arrow.clockwise`, `doc`, `square.grid.2x2`) are not checked against the symbol set; a missing symbol
  shows no icon.
- Import files are read as UTF-8 with invalid bytes replaced. No other text encodings (UTF-16, Latin-1) are handled.
