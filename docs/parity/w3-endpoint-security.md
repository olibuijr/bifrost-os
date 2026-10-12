# Endpoint Security parity, wave 3

Three gap rows of the Endpoint Security framework now have pure models in `bifrost_app` (`springboard/sdk/src`).
Each model covers the event or response shape and its rules. None of them is wired into the shell, an app or a
kernel hook, so every row is `partial`, not `parity`.

## What was built

| Module | Rows | What it models |
|---|---|---|
| `springboard/sdk/src/file_metadata.rs` | File metadata events | `Attributes` snapshot (permission bits, uid, gid, flags, extended attributes, ACL text). `diff(path, before, after)` returns mode, owner (uid and gid together), flags, extended attribute delete and set (sorted by name), and ACL set or clear, in a fixed order. |
| `springboard/sdk/src/auth_events.rs` | Authorization events and responses | `Policy` with ordered `Rule`s (operation, whole-component path prefix, source process), first match wins, with a default. `AuthQueue` holds requests, accepts one answer per request, and applies `DeadlineMode` (fail closed, fail open, kill) to requests past their deadline. |
| `springboard/sdk/src/identity_events.rs` | User and group ID change events | `Credentials` with real, effective and saved IDs for user and group. The six ID-setting calls apply the Linux credential rules (refusal for unprivileged callers) and return the matching event. Privilege is approximated as effective ID 0. |

Each module has its own `#[cfg(test)] mod tests`. The expected values were worked out by hand from the rules in the
code (for example, the saved ID after `setreuid(None, Some(1001))` from real 1000 is 1001, and an unprivileged
`setuid(0)` is refused).

`springboard/sdk/src/lib.rs` gained three `pub mod` lines (`file_metadata`, `auth_events`, `identity_events`).

## Apple API names mirrored

- `es_event_setowner_t`, `es_event_setmode_t`, `es_event_setflags_t`, `es_event_setextattr_t`,
  `es_event_deleteextattr_t`, `es_event_setacl_t` (File Metadata Event Types)
- `es_auth_result_t` (`ES_AUTH_RESULT_ALLOW`, `ES_AUTH_RESULT_DENY`), `es_deadline_miss_mode_t`
  (`ES_DEADLINE_MISS_MODE_FAIL_CLOSED`, `ES_DEADLINE_MISS_MODE_FAIL_OPEN`, `ES_DEADLINE_MISS_MODE_KILL`), the
  Authorization events of Endpoint Security
- `es_event_setuid_t`, `es_event_seteuid_t`, `es_event_setreuid_t`, `es_event_setgid_t`, `es_event_setegid_t`,
  `es_event_setregid_t` (User and Group ID Types)

The official pages are the DocC JSON for each symbol, read with `tools/parity/docc.py`, and the framework landing page
`https://developer.apple.com/documentation/endpointsecurity`.

## REA evidence

| Page | evidence_id |
|---|---|
| Endpoint Security (framework landing) | `ev_b1022d5cd4486cb623f1a7d721f44bdebd42a7e615467e996cade90301032341` |
| `es_event_setowner_t` | `ev_e558d1403143406eeaf675b51b6c4baf23f2063deafb9801201cf7cb6e812f92` |
| `es_auth_result_t` | `ev_c5334cedb1167888c78d0da020c97990f72be0270e82b32f51aafd24130d38eb` |
| `es_event_setreuid_t` | `ev_4b50a784b52900906d8e959e492e22559fa35be6d4799ceaced08f67474e4601` |

## Rows closed

These rows move from `gap` to `partial` in `docs/parity-matrix/data/endpoint-security.json`. The lead updates the
matrix.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| endpoint-security | File metadata events (xattr, ACL, flags, mode, owner, stat, readdir) | partial | springboard/sdk/src/file_metadata.rs |
| endpoint-security | Authorization events and responses | partial | springboard/sdk/src/auth_events.rs |
| endpoint-security | User and group ID change events | partial | springboard/sdk/src/identity_events.rs |

## Not covered

- File metadata: stat and directory-listing records are not events, so they are not modelled. The model is a diff of
  two snapshots, not a hook on a change, and nothing reads the file system.
- Authorization: nothing asks a client before an operation runs. The queue and policy are not connected to the
  docbroker or the launch sandbox, and there is no cache for responses.
- Credentials: the privilege check is approximated as effective ID 0, and no capability check is modelled. No
  process is switched by the model.
- Still gaps in the matrix: login and session events, memory mapping, socket and pseudoterminal events, and code
  signing status.
