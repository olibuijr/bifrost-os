# Service Management parity, wave 3

Official reference: https://developer.apple.com/documentation/servicemanagement (framework page and the
`SMAppService` class page, https://developer.apple.com/documentation/servicemanagement/smappservice).
REA evidence of the rendered pages:

- `ev_c884224fcdcadf51868dffdfc43353d0a3db19c7964885cec36e4f01323c23ed` (Service Management framework page)
- `ev_2b1b264277a469d37e09f0e0aaf988b86315168316323375b70133d319fa8a54` (SMAppService class page: `agent(plistName:)`,
  `daemon(plistName:)`, `loginItem(identifier:)` return `Self`; `register()` throws)

## What was built

All code is in `springboard/sdk/src/service_management.rs` (extended, no new module). Pure Rust, no shell change: the
new handles never send a request to the shell.

- `SMAppService` is now a handle with four kinds: `main_app()` (unchanged behaviour), `login_item(identifier)`,
  `agent(plist_name)`, `daemon(plist_name)`. It derives `Clone` and `Debug` instead of `Copy` because the other kinds hold a name.
- `status()` returns the last shell status for the main app; every other kind returns `NotFound`, since no helper
  bundle, launch agent or launch daemon ships in an app bundle.
- `try_register()` / `try_unregister()` return `Result`. For the main app they send the request as before and return
  its id. For a login item they return `ServiceError::JobNotFound`. For an agent or daemon they return
  `ServiceError::InvalidPlist` when the name is not a `.plist` name (no `/`, a `.plist` suffix with something before it),
  otherwise `ServiceError::JobPlistNotFound`. Nothing is sent in the error cases.
- `register()` and `unregister()` keep their signature; they return the `try_` id, or 0 when no request was sent.
- `refresh()` sends `Query` for the main app and nothing (returns 0) for the other kinds.
- `ServiceError`: the eleven constants of the official "Service Management Errors" list
  (`AlreadyRegistered`, `AuthorizationFailure`, `InternalFailure`, `InvalidPlist`, `InvalidSignature`,
  `JobMustBeEnabled`, `JobNotFound`, `JobPlistNotFound`, `LaunchDeniedByUser`, `ServiceUnavailable`, `ToolNotValid`).
  It implements `Display` with the official descriptions and `std::error::Error`, and `ServiceError::ALL` lists them.
  The documentation gives no numeric codes, so none are invented.

## Limits, stated plainly

- The agent, daemon and login-item handles are answers, not running services. Nothing is loaded from a bundle, and no
  launch agent or daemon is started. A later change would need a bundle format and a `bifrostd` job API.
- The shell reply still carries only a status. A failure of the main app is not reported with a `ServiceError`.
- `JobMustBeEnabled` and `JobPlistNotFound` have no description in the official list; their `Display` texts are our wording.
- Not built: embedded helper executables, legacy plist authorization check (`statusForLegacyPlist(at:)`),
  `unregister` with an error completion (already `partial` in the matrix).

## Tests

`cargo` was not run (wave rule: no builds). The module's tests, written from the rules above:

1. `calls_go_to_the_shell_and_status_follows_the_report` (existing, unchanged).
2. `main_app_try_register_returns_the_request_id_it_sent`: `try_register` returns the id of the one message sent.
3. `other_kinds_report_not_found_and_do_not_read_the_shell_status`: login item, agent, daemon and an invalid plist name report `NotFound`.
4. `login_item_register_fails_with_job_not_found_and_sends_nothing`: `JobNotFound` from register and unregister, and no message sent.
5. `agent_and_daemon_names_are_checked_before_the_bundle_lookup`: `com.example.agent.plist` gives `JobPlistNotFound`;
   `com.example.agent`, `dir/job.plist`, `.plist` and `""` give `InvalidPlist`; nothing sent.
6. `refresh_for_other_kinds_sends_nothing`.
7. `service_errors_have_the_official_texts_and_are_all_distinct`: 11 variants, two exact texts, 11 distinct `Display` strings.

## Matrix rows closed (status `partial`, pending the lead's central update)

| Framework slug | Feature (as in data/service-management.json) | New status | BifrOSt path |
|---|---|---|---|
| service-management | Login item by bundle identifier | partial | springboard/sdk/src/service_management.rs (SMAppService::login_item, try_register) |
| service-management | Launch agents | partial | springboard/sdk/src/service_management.rs (SMAppService::agent, try_register) |
| service-management | Launch daemons | partial | springboard/sdk/src/service_management.rs (SMAppService::daemon, try_register) |
| service-management | Service Management error constants | partial | springboard/sdk/src/service_management.rs (ServiceError) |

Not closed: Embedded helper executables, Legacy login item authorization check, Privileged helper authorization,
Login item enablement (deprecated).

## Notes for the lead

- `docs/bifrost-ui.md` (section "Login items and menu-bar extras") still lists `loginItem(identifier:)`, `agent` and
  `daemon` as gaps; it should now say they are handles that answer `NotFound` and `ServiceError` values. Not edited here (shared doc).
