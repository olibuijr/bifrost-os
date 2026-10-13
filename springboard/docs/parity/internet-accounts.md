# Internet Accounts (round 75)

Slice: Settings > Internet Accounts, a shared account list that Mail, Calendar, Contacts and Reminders read, CalDAV and
CardDAV sync for Calendar and Contacts, and credentials in the passwords group.

Nothing here is built or run on Midget yet. The code was written to compile by reading; the lead builds, tests and
checks on Midget.

## Reference evidence

- `~/Projects/macos-ref/extract/ref27/System/Library/ExtensionKit/Extensions/InternetAccountsSettingsExtension.appex/Contents/Info.plist`
  (read with `plistlib`): extension point `com.apple.Settings.extension.ui`, bundle id
  `com.apple.Internet-Accounts-Settings.extension`. Only `Info.plist` was extracted; the `en.lproj` label tables are
  listed in `ref27-index.tsv` but not extracted, so the row labels come from the account types themselves, not the strings.
- Account framework inventory: `docs/parity-matrix/data/accounts.json` (feature rows listed below).
- No REA evidence id was taken for this slice.

## Rows

| Matrix entry (accounts.json) | Status after this slice | BifrOSt path |
|---|---|---|
| Account store (ACAccountStore) | partial: one shared list across apps (file), no access prompt | `ui/src/internet_accounts.rs` (model, file), `sdk` reads via `bifrost_app::controls::internet_accounts` |
| Account objects (ACAccount) | partial: identifier, username, kind, services; no full name or description | `ui/src/internet_accounts.rs` `Account` |
| Account types and access grants (ACAccountType) | partial: Mail, CalDAV, CardDAV kinds; the per-service switch is the grant | `Kind`, `Service` in `ui/src/internet_accounts.rs` |
| Credentials (ACAccountCredential) | partial: password only, in the passwords group (site = host, account = username); no OAuth | `shell/services/keychain_groups.rs` `save`, `sdk/src/passwords.rs` |
| Sign-in and account creation prompts | partial: Add Account is three one-field prompts in Settings | `shell/internet_accounts_page.rs`, `shell/forms.rs` |

Entries left as they are: Credential renewal, OAuth tokens, Errors (ACErrorCode). Nothing is marked fully `parity`.

## What is done

1. Shared list (`ui/src/internet_accounts.rs`): IMAP/SMTP (mail), CalDAV and CardDAV accounts; per-account services
   (Mail; Calendars and Reminders for CalDAV; Contacts for CardDAV); a text file at
   `$XDG_DATA_HOME/bifrost/internet-accounts.ini` (or `~/.local/share/...`) with no password in it; validation; `serving`
   lists the accounts a service reads. Unit tests cover round trip, validation, service rules, URL hosts, file save and load.
2. DAV protocol (`ui/src/internet_accounts/dav.rs`): well-known paths, principal, home-set and collection requests,
   calendar and address-book queries, a multistatus reader (namespaces ignored, 404 propstats dropped, entities decoded),
   and UTC timestamps. Unit tests cover them.
3. SDK client (`sdk/src/dav_client.rs`): blocking discovery (principal at the URL, then at the well-known path; home set;
   collections of the right kind) and reads of events and cards, with Basic auth. The transport is a `send` function;
   `session_send` uses the URL session. Tests use canned answers.
4. SDK sync driver (`sdk/src/dav_sync.rs`): `Sync` asks the passwords store for the password, runs the walk on a background
   thread, and returns the result from `poll`. An account syncs at start and every 15 minutes.
5. Settings pane (`shell/internet_accounts_page.rs`, wired in `shell/pages.rs` and `shell/settings.rs`): one group per
   account with a switch per service, Remove Account, and Add Mail, Calendar, Contacts Account. Add is three prompts
   (address, user name, password). Mail takes IMAP on 993 (TLS) and SMTP on 587 (STARTTLS) from the domain of the address.
6. Mail reads the list (`apps/mail-desktop/src/accounts.rs`, `mail_accounts`). The private account module is deleted;
   `valid_address` comes from the shared module. Version 0.3.0.
7. Calendar syncs CalDAV calendars (`apps/calendar-desktop/src/main.rs`): finished events are imported with `import_ics`.
   Version 1.12.0.
8. Contacts syncs CardDAV address books (`apps/contacts-desktop/src/main.rs`, `contacts.rs`): `merge_card_texts` is the
   vCard merge, now shared with the file import (the import's loop moved into it). Version 1.8.0.

## What is still a model, or missing

- Reminders reads nothing yet. The Reminders service can be switched on for CalDAV accounts, but no VTODO sync exists.
- Mail has no IMAP or SMTP connection in the app (the protocol parsers already existed; no network layer). `mail_accounts`
  only returns the accounts; no sync happens.
- Edit: an account cannot be renamed, its servers changed, or its password replaced from the pane. Only add, switch, remove.
- Account access prompts for other apps (ACAccountStore's `requestAccess`) are not built; any app with the passwords group
  can read the login.
- Calendar imports each calendar object with its own `import_ics` call, and every call rewrites the store. Fine for a
  personal calendar; a large one should batch it.
- Sync errors are not shown to the user in Calendar or Contacts (the result is dropped). The settings pane shows only
  failures of its own changes.
- The sandbox: the shell and apps read `$XDG_DATA_HOME` or `~/.local/share/bifrost/internet-accounts.ini` directly. Check on
  Midget that an app with `storage` can read and write there, or move the file behind a storage service.

## Midget checks needed

- Settings > Internet Accounts: the empty page, Add Mail (three prompts, the password sheet is secure), a CalDAV account
  with its two switches, Remove, and the red note for a refused change.
- A real CalDAV and CardDAV server: discovery against the well-known path, one calendar and one address book, the 15 minute
  repeat, and that a second sync does not duplicate contacts (merge skips duplicates) and events (import skips matches).
- Passwords page: the saved password appears under the server host, and Remove deletes it.
- Calendar and Contacts open with no account and with a bad one, without a crash or a blank list.
- The list file: created with the right mode under the user's data directory, and not readable by other users.
