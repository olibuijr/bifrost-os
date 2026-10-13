# Keychain Access and Passwords (slice keychain-passwords, round 75)

Scope: the login keychain store used by Mail, Wi-Fi and Internet Accounts; the Passwords app; certificates and secure
notes; search; reveal after authentication; autofill into the browser and system password prompts.

## Evidence

- Reference index: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Applications/Passwords.app`
  (Info.plist at 4475 bytes, with a LoginItems menu-bar extra and SharedWebCredentials) and
  `System/Library/CoreServices/Applications/Keychain Access.app` (Info.plist at 5177 bytes). Only names and sizes were
  used.
- The extracted bundles are not in `~/Projects/macos-ref/extract/ref27/` yet (only the Mail, Notes and other apps
  are there). No `inspect_plist` or `decode_interface_builder` run was made for this slice, so no REA evidence ids are
  recorded. Re-check after the lead's next extraction and record the Info.plist and nib findings here.

## Rows and BifrOSt paths

| Area | API (neutral name) | BifrOSt path | State |
|---|---|---|---|
| Login keychain store (per app and per group) | keychain items `(service, account)` | `springboard/src/keychain.rs` (`Store::open`, `Store::open_group`), SDK `springboard/sdk/src/keychain.rs`, `keychain_groups.rs` | Done before this round (unchanged) |
| One credential store API | item kinds, names, listing, secret, removal | `springboard/src/credentials.rs` (new) | Done: the one definition used by the shell and the SDK |
| Settings > Passwords page | list logins, reveal after passcode, copy, delete | `springboard/src/shell/passwords_page.rs`, `settings.rs` (`passwords_act`) | Migrated to `credentials::{logins, secret, remove}`; the duplicate helpers in `shell/services/keychain_groups.rs` are deleted |
| Saving through the group | add, update, get, delete, list, changed event | `springboard/src/shell/services/keychain_groups.rs`, SDK `passwords.rs` | Unchanged wire API; the SDK re-exports the kinds |
| Secure notes | save, read, delete | SDK `passwords::save_note`; store `note:<title>` with account `secure-note` | Done in the model and the store; the app shows and deletes notes; creating one from the app needs a text editor and is still missing |
| Certificates | save, read, delete | SDK `passwords::save_certificate`; store `cert:<label>` with account `certificate` | Same as notes: stored, listed, revealed (PEM text), deleted; no certificate parsing or trust settings yet |
| Search | word match on name and account | `apps/passwords-desktop/src/model.rs` (`matches`, `filter`) | Done, now per kind tab |
| Show password after authentication | passcode before reveal | Settings page: `passcode::Store` overlay (`passwords_act`) | Done in Settings. The Passwords app still reveals without the passcode: a shell call for an app-side passcode prompt is missing |
| Autofill into the browser and system password prompts | fill request | SDK `passwords::fill`, no browser or system prompt hook | Missing: the browser app does not ask the store, and no system prompt is wired |

## Decisions

- A kind is carried by the keychain service prefix (`note:`, `cert:`). A site name cannot contain a colon, so a login
  is never read as a note or certificate. A login saved with such a site would be listed as a note; the SDK and the
  Passwords import both refuse to create one (the import counts it as skipped; the SDK `save` does not check yet).
- Secrets stay in the shell store. The app reads one at a time, only when an item is revealed, copied or exported.
- The list model is one `Item` in the app (`model::Item`), built from the listing row through `Item::new`.

## Still to check on Midget

- Tabs and search per kind; a note or certificate revealed and copied; delete from each tab.
- Settings > Passwords still lists logins only, and its reveal still asks for the passcode.
- Import of a CSV whose site starts with `note:` is counted as skipped.
- Keychain Access app (certificates and secure notes as a full app): not started; needs the extracted bundle.
