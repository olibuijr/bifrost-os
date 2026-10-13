# App Services SDK slice (parity round 75, slug sdk-app-services)

Rows advanced: **Contacts** (Partial, 50%), **Address Book** (Partial, 50%). Contacts UI is unchanged (no picker
surface reaches apps yet). The App Services gaps for notifications and file providers were checked and are already
partly built: `bifrost_app::usernotif` has a settings query (`Event::Settings`, authorization flags), a repeating
request, and categories; `bifrost_app::file_provider` exists. Those rows still list gaps (attachments, push, a
provider domain model), so they stay open.

## Contacts and Address Book: access, fetch and search

Official reference: `CNContactStore` (access request, `unifiedContacts(matching:keysToFetch:)`, the key list).
Clean-room evidence: the names only, from the shipped Contacts framework inventory. No copied files.

Models (already in `springboard/ui/src`, now used end to end):
- `contact_access.rs`: `Access` (every contact, a picked set read-only, restricted, or none), `Choice`, `Status`.
  Added `Status::name` and `Status::from_name` (wire names `not_determined`, `restricted`, `denied`, `authorized`,
  `limited`).
- `contact_fetch.rs`: partial records (`Keys`, `Record`, `Partial`, `project`). Added `Key::name` and `Key::from_name`
  (`given_name`, `family_name`, `organization`, `phones`, `emails`, `address`, `note`) and `Record::get`.

Shell (new): `springboard/src/shell/services/contacts.rs`, the `contacts` service.
- `access {}` returns `{status, picked}`.
- `request {}` returns `{status}` and changes nothing, so an app cannot grant itself access.
- `fetch {keys, query?}` returns `{contacts: [{id, <key>: value | null}]}`, sorted by name, only the contacts the app
  may read, filtered by `query` (any text of the contact). Without a read grant it is an error. The book is loaded
  only for a fetch by an app with a grant.
- `answer_prompt(app, choice)` and `change_by_user(app, choice)` are the two entry points for the access sheet and
  for Settings. Neither is drawn yet (see gaps).
- The store has one name per contact: the last word is the family name, the rest is the given name.

SDK (new): `springboard/sdk/src/contacts.rs`, `bifrost_app::contacts`.
- `access`, `request`, `fetch(ctx, keys, query)` send the calls.
- `decode_status`, `decode_picked`, `decode_contacts` read the replies (`Vec<Record>`).
- `preview(choice)` applies the same rules as the shell, for a test or a sheet preview.

Shared path: `contactstore::store_path(data_dir)` is the one place the book's file is named. The desktop search
(`quicksearch.rs`) now calls it instead of the inline path (adopt and delete).

Tests (not run here, the lead builds): `contact_access` (status names), `contact_fetch` (key names, `Record::get`),
`services/contacts.rs` (grant rules, projection, sort, query, blank values as null, name split), `sdk/contacts.rs`
(reply decoding, preview rules).

## Gaps that remain (Midget checks and next steps)

- No access sheet: `request` does not queue a prompt yet, so every app reads `not_determined` and cannot read until
  Settings or a sheet calls `change_by_user` / `answer_prompt`. The privacy service's `queue_prompt` is the pattern to
  copy. Needs a Settings page row and a sheet before this is usable on Midget.
- Grants reset on shell restart (thread-local, like `privacy`). They should persist with the prefs store.
- Writes: no `save`, `add` or `delete` call. A grant is read-only for the SDK today.
- Contacts UI (picker and card for apps): not in the SDK. `bifrost_ui::contact_picker` exists but is not wired.
- The store keeps one phone, one email and one address. Multi-value fields and labels are not sent to apps.
- No Midget check of the service yet. The shell must be built and the `contacts` service called from a test app.

## Files touched

- `springboard/ui/src/contact_access.rs`, `springboard/ui/src/contact_fetch.rs` (wire names, `Record::get`)
- `springboard/src/shell/services/contacts.rs` (new), `springboard/src/shell/services/mod.rs` (module and registration)
- `springboard/src/shell/contactstore.rs` (`store_path`), `springboard/src/shell/quicksearch.rs` (caller migrated)
- `springboard/sdk/src/contacts.rs` (new), `springboard/sdk/src/lib.rs` (`pub mod contacts;`)
