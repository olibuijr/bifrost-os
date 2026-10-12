# Core Services parity, wave 3

Three gap rows of the Core Services framework, built as pure Rust (std only, no I/O, no hardware, no network).
Each module is a model with tests; none is wired into a shell surface yet.

## What was built

- **Role handlers** (`springboard/ui/src/launch.rs`, extends the default-application registry). A `Role` enum
  (viewer, editor, shell) and per-role declarations on `Handler::role(role, uti)`. The registry answers which apps
  act in a role for a type (nearest declaration first, then registration order), keeps the user's choice per
  (type, role), and stores it in the prefs lines as `role <uti> <role> <app>`. Role declarations are independent of
  `opens`, so the type default (`default_for_type`) does not change.
- **Backup exclusion** (`springboard/core/src/backup_exclude.rs`, new). A set of absolute paths the backup skips.
  Excluding a folder covers its contents by whole path component. Relative paths and paths with line breaks are
  refused. The set is stored one path per line, sorted.
- **Quarantine properties** (`springboard/core/src/quarantine.rs`, new). A record of how an item arrived: kind
  (web download, other download, email, message, calendar, other attachment), agent bundle id and name, time in Unix
  seconds, and optional data and origin addresses. Written and parsed as `key=value` lines; a missing key or a bad
  kind or time is reported by name.

## Official API names mirrored

Launch Services role handlers (`LSCopyAllRoleHandlersForContentType`, `LSCopyDefaultRoleHandlerForContentType`,
`LSSetDefaultRoleHandlerForContentType`, `LSRolesMask`) and the quarantine properties keys
(`kLSQuarantineAgentBundleIdentifierKey`, `kLSQuarantineAgentNameKey`, `kLSQuarantineTimeStampKey`,
`kLSQuarantineTypeKey`, `kLSQuarantineDataURLKey`, `kLSQuarantineOriginURLKey`, and the six
`kLSQuarantineType*` values). Backup Core (`CSBackupSetItemExcluded`, `CSBackupIsItemExcluded`).

Official pages used: https://developer.apple.com/documentation/coreservices/launch_services,
https://developer.apple.com/documentation/coreservices/backup_core,
https://developer.apple.com/documentation/coreservices.

## REA evidence

- Launch Services: `ev_3b000c6a66586a5a54d2c7d609e7532c53455d5a3496b6494a105663787510b2`
- Core Services: `ev_38783bb15a408b5c8048ec0fd667b42dc2ce9f76f3d6e0108d38f5fdf1fd0237`
- Backup Core: `ev_1c4466b667a1bb74c1fb584204feae19cacf3368a04c182448564f8b0be3a2b8`

## Matrix rows closed

These rows move from `gap` to `partial` (the lead updates the matrix centrally).

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-services | Role handlers (viewer, editor, shell) | partial | springboard/ui/src/launch.rs (`Role`, `Handler::role`, `Registry::default_for_role`) |
| core-services | Backup exclusion | partial | springboard/core/src/backup_exclude.rs (`BackupExclusions`) |
| core-services | Quarantine properties | partial | springboard/core/src/quarantine.rs (`Quarantine`, `QuarantineKind`) |

## Not done

- No shell surface reads the quarantine record before opening an item, and nothing writes it on download.
- Backup exclusion is not applied by any backup job, and nothing persists the set outside the module's text form.
- Role handlers are not shown in an Open With menu per role; the menu still lists the type candidates.
- Shared file lists, Apple Events, Dictionary lookup, event IDs and file metadata queries remain gaps.
