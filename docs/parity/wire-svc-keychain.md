# Wiring: keychain access groups (`keychain-groups` service)

Branch `wire/svc-keychain`. This wires the keychain access-group gap (`kSecAttrAccessGroup`: items shared between
apps) into the shell and the app SDK. It is a wiring change, not a new parity model: no model was added, because the
parity matrix had none for access groups. The gap was only recorded as a note (`docs/apple-parity-matrix.md`,
Security and Authentication Services rows, and `docs/parity-matrix/data/security.json`,
`docs/parity-matrix/data/authentication-services.json`, which this branch does not edit).

## What is wired

| Piece | Where |
|---|---|
| Access-group store (one file per group, same master key) | `springboard/src/keychain.rs` (`Store::open_group`, `group_ok`, `Error::from_code`, `hex`/`unhex` made public) |
| Manifest field `keychain_groups = ["..."]` | `springboard/src/shell/apphost.rs` (`Manifest::keychain_groups`, parsed in `Manifest::parse`) |
| Shell service `keychain-groups` (registered in the services list) | `springboard/src/shell/services/keychain_groups.rs`, one line in `services/mod.rs` |
| SDK client (typed calls, reply decoding, `changed` event) | `springboard/sdk/src/keychain_groups.rs`, one line in `sdk/src/lib.rs` |
| Docs | `docs/bifrost-ui.md`, Keychain section |

The wire protocol (`springboard/proto`) is unchanged. The calls use the existing `ToShell::ServiceCall` and
`ToApp::ServiceReply` / `ServiceEvent` channel.

## How an app reaches it

1. The manifest declares the permission and the groups:

   ```toml
   permissions = ["keychain"]
   keychain_groups = ["is.example.keys"]
   ```

   Groups count only with the `keychain` permission. Without it the service answers `NotPermitted` for every call.
2. The app calls the SDK client (`bifrost_app::keychain_groups`):
   - `add(ctx, group, service, account, secret)`, `get(...)`, `update(...)`, `delete(...)`, `groups(ctx)`. Each returns a
     request id.
   - In `App::service_reply`, `keychain_groups::decode(result)` gives `Result<Vec<u8>, Error>` (the secret for `get`).
     `decode_groups` reads the `groups` answer.
   - In `App::service_event`, `keychain_groups::changed(service, event, body)` gives the `(group, account)` another
     member just changed (event `changed`, sent to every member on the next shell tick).
3. Errors are the keychain's own `Error` (`DuplicateItem`, `ItemNotFound`, `NotPermitted`, `Param`, `Io`), with the
   same OSStatus codes as the app keychain.

Store layout: `<shell root>/keychain/groups/<group>.kc`, encrypted with the same master key as the app stores and keyed
by the group. Group names use the app-id alphabet (letters, digits, `.`, `-`, `_`; no leading dot; at most 128 bytes),
so a name cannot escape the directory.

## Reference behaviour (official documentation)

- Symbol page `kSecAttrAccessGroup`, read with `python3 -I tools/parity/docc.py security/ksecattraccessgroup`
  (the DocC summary: "A key with a value that's a string indicating the access group the item is in").
- Rendered page, REA evidence `ev_d841fc09eb19d111145d99ce7601cd842cbb42fcb8f1bb48d35e3ee3efcc7ebd`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/security/ksecattraccessgroup`). The rules it
  states: an app reaches an item only through a group it belongs to; an item added without a group goes to the app's
  first group; a group the app is not in is an error, and so is the empty group; `update`, `delete` and search look
  across all the app's groups unless one is named.
- Existing Security collection evidence, cited by the parity matrix: `ev_3bcba33c922f0c6a1f91f930d57aed415e4f75de88d819ed415661efaa4fcee6`.

Where this wiring differs from the reference:
- The group is required on every call. The default is not "the first group", and there is no search across groups.
  The reference's default search across groups is the more useful behaviour and is not built.
- Calls are asynchronous (the app's reply arrives later), as the app keychain already is.
- A group is a manifest list only. There is no entitlement signing, so the shell decides membership from the installed
  manifest, not from a signed entitlement.

## How to verify on the test PC (Midget)

Midget is the only test machine (no VM builds, per the owner's instruction). No first-party app declares
`keychain_groups` yet, so verification needs a test app built with the SDK:

1. Build an app with `permissions = ["keychain"]` and `keychain_groups = ["is.olibuijr.wire.test"]`, and a second app
   with the same group. Publish both (`tools/pc-app.py build|publish <dir>`) and install them on Midget with
   `ssh midget` then `/usr/bin/springboard --install-store-app <id>`.
2. Run app A, which calls `keychain_groups::add` with a value. Run app B, which calls `keychain_groups::get` for the same
   `(group, service, account)`: the secret must come back. Check the app's own keychain (`keychain::get` with the
   same service and account) does not see it.
3. Remove the `keychain` permission from app B and run it again: its call must answer `NotPermitted`.
4. Watch app B's `service_event` while app A writes: a `changed` event with the group and account must arrive.
5. On Midget, the group files are `/var/lib/springboard/keychain/groups/<group>.kc` and are mode 0600 in a 0700 directory.

Nothing above has been run yet. The branch was written without builds, so the lead's build and the Midget steps are the
first real check.

## Tests

Written, not run (no builds on this branch):
- `springboard/src/keychain.rs`: group file shared by name and separate from app files, group-name refusal, `from_code`
  round trip, hex round trip.
- `springboard/src/shell/services/keychain_groups.rs`: `groups`, sharing between members, refusal outside the group
  (nothing written), a change reported only for a successful mutation, a bad group name as a `Param` status, malformed
  requests as service errors, manifest parsing of `keychain_groups`, and the permission gate.
- `springboard/sdk/src/keychain_groups.rs`: request shape, status-to-error decoding, malformed answers, the groups answer,
  and the `changed` event.

## Remaining gaps

- No first-party app declares `keychain_groups` yet, so no shipped app shares items.
- No Settings or UI surface for a user to see or remove shared items; the group files are shell-private.
- No group search (`kSecMatchLimitAll`), no default group, no item attributes beyond service and account.
- The shell still runs as root and the master key is on the same disk (the existing keychain limitation).
- Apple's `AutoFill` / shared credential prompt is not built.
