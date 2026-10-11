# Theme 01: identity services (users and groups) and access control

Branch `parity/theme-01`. Pure Rust, std only, in the bifrost-ui crate. No builds were run for this theme (by rule);
the modules were checked by reading and by `python3 tools/check-names.py` (exit 0).

## What was built

- `springboard/ui/src/identity.rs`: a user and group directory (`Directory`). Users have a numeric id, name, full name,
  primary group, home and shell. Groups have a numeric id, name and member list. Lookups by name and by id;
  `groups_of` returns the primary group first, then memberships in id order; `is_member` counts the primary group.
  Add, remove and member changes check names (`[a-z_][a-z0-9_.-]*`, up to 32 bytes), unique ids and names, fields
  with no colon or line break, and a primary group that exists. A group that is still someone's primary group cannot
  be removed. `load(passwd, group)` reads the colon-separated text forms; `to_passwd` and `to_group` write them back
  (round trip tested). Nothing reads the host's own files.
- `springboard/ui/src/access_control.rs`: keychain-style access rules. `Constraint` (device passcode, any biometry,
  current biometry set, user presence, watch), `Expr` (constraint, `and`, `or`, validated for empty conjunctions and
  depth at most 8), `Presence` (what the user just presented, supplied by the caller), and `AccessControl::allows`.
  `RightStore` keeps authorization rights granted by name, each valid until revoked or for a number of seconds, with
  `state`, `revoke` and `prune`. Time is passed in by the caller; the module reads no clock.
- `lib.rs`: two `pub mod` lines (`identity`, `access_control`).

Tests: 8 in `identity.rs` and 9 in `access_control.rs`, all as behaviour checks (lookups, refused duplicates and bad
names, primary-group rules, membership ordering, text round trip, line-numbered parse errors, capacity limits,
constraint truth tables, expiry boundaries, prune, renewal, capacity).

## Official API names mirrored

Verified from the official DocC JSON and the rendered page:

- `SecAccessControl` (class, "An opaque type that contains information about how a keychain item may be used").
- `SecAccessControlCreateFlags` constraints: `devicePasscode`, `biometryAny`, `biometryCurrentSet`, `userPresence`
  (biometry or passcode), `watch`; conjunctions `and` and `or` ("all constraints must be satisfied" / "at least one").
  Mirrored as `Constraint` and `Expr`.

Named in the matrix but **not** fetched: the Identity Services article (`docc.py` returned 404 for the paths tried),
`LARight`, `LARightStore`, `LAPersistedRight`, `LASecret` (404 for the paths tried) and `Authorization` (404). For these
the matrix row text is the only source, so `identity.rs` and `RightStore` follow the row wording and no Apple
signatures are claimed.

## REA evidence

- `ev_40c4d23a0c79d824886062457cb7483623e5b8d7832ebf4032aed83051ca4f7b`: rendered page of `SecAccessControl`
  (https://developer.apple.com/documentation/security/secaccesscontrol).
- `ev_9b086ac3f8831fd6cc75b755b9543dbe2744d56ca5ce7f0a590baf9b21f86086`: rendered page of
  `SecAccessControlCreateFlags` (https://developer.apple.com/documentation/security/secaccesscontrolcreateflags).

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| core-services | Identity services (users and groups) | partial | springboard/ui/src/identity.rs (`Directory`) |
| local-authentication | Keychain items protected by user presence or biometrics | partial | springboard/ui/src/access_control.rs (`Constraint`, `Expr`, `AccessControl`) |
| local-authentication | Persisted rights and secrets (LARight, LAPersistedRight, LASecret, LARightStore) | partial | springboard/ui/src/access_control.rs (`RightStore`; rights only) |
| security | Authorization Services and privileged operations | partial | springboard/ui/src/access_control.rs (`RightStore`; no privileged execution) |

## Limits (why these are partial)

- The keychain in `springboard/src/keychain.rs` does not call `AccessControl::allows` yet. Items are still readable
  whenever the shell runs, so user presence is modelled, not enforced.
- No secret storage is attached to `RightStore`, and nothing asks the user to grant a right. No shell or SDK API
  exposes either module, so apps cannot use them yet.
- `Directory` is not connected to the shell's sign-in (`springboard/src/privd/auth.rs`) or to the cloud accounts. It
  is a model that can read and write the two text formats.
- Nothing here reads the biometry sensor or the passcode; the caller reports them in `Presence`.
