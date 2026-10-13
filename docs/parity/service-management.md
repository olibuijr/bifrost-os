# Service Management parity

Apple's Service Management framework lets an app register login items, launch agents and launch daemons. Matrix row:
`docs/parity-matrix/data/service-management.json`. Earlier wave notes: `docs/parity/w3-service-management.md`.

## What is built

- Login item for the app itself (`SMAppService.mainApp`): `springboard/sdk/src/service_management.rs`. Register,
  unregister, refresh and status go to the shell over the login-items request, and the shell keeps the list
  (`springboard/src/shell/extras.rs`, `springboard/src/shell/pages.rs`). Parity row; tested on Midget earlier.
- `unregister(completionHandler:)` (`unregister_with`), `openSystemSettingsLoginItems()`, and the eleven `kSMError*`
  values as `ServiceError` (`code`, `from_code`, `to_error` in `SMAppServiceErrorDomain`).
- `springboard/sdk/src/service_bundle.rs` (new, pure, no I/O): the app-bundle side of the framework.
  - Embedded login-item helpers live in `Contents/Library/LoginItems/<Name>.app`. `helper_from_entry` builds a
    `HelperItem` from a bundle path and the helper's `CFBundleIdentifier`; `helper_path_ok` rejects anything not
    directly inside that directory, nested paths and parent steps.
  - `find_login_item(identifier, items)` answers `loginItem(identifier:)`: `JobNotFound` for an empty identifier, one
    with a slash, or one that no helper declares.
  - `helper_status(identifier, items, enabled)` gives `NotFound` when no helper has the identifier, `Enabled` when the
    caller lists it as enabled, and `NotRegistered` otherwise.
  - `locate_job_plist(kind, name, bundle_files)` answers `agent(plistName:)` and `daemon(plistName:)` against the
    bundle's file list (`Contents/Library/LaunchAgents`, `Contents/Library/LaunchDaemons`). A name that is not a
    `.plist` name is `InvalidPlist` (the check is `service_management::valid_plist_name`, shared); a missing file is
    `JobPlistNotFound`.

## What is not wired

- The `SMAppService` handles for `loginItem`, `agent` and `daemon` still answer `NotFound` and the same errors as before.
  They do not scan the app bundle, so `service_bundle` is not called from them yet.
- No helper or job is started. The shell does not list embedded helpers in Login Items, and bifrostd is not
  exposed to apps as a launch-agent supervisor.
- `statusForLegacyPlist(at:)` stays as in wave 5: the pure model exists, and BifrOSt has no earlier plist format.

## Next steps

1. Give the app's bundle reader (`springboard/sdk/src/cf_bundle.rs`) a way to list `Contents/Library/LoginItems`
   and read each helper's identifier, then pass the result to `find_login_item` and `helper_status`.
2. Have the shell report which helpers are enabled, and show them on the Login Items page.
3. Test on Midget, then move the rows to parity.

## Verification

Host unit tests only, in `springboard/sdk/src/service_bundle.rs` and `springboard/sdk/src/service_management.rs`.
Not yet built or run: the lead runs the build and tests. Not yet verified on Midget.
