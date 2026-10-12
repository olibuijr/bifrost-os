# Wave 5, theme-10: Preferences session for system configuration

## What was built

`springboard/core/src/pref_session.rs` (`PrefsSession`, std only, pure model, registered in `springboard/core/src/lib.rs`).

A system-wide configuration store with three copies of its values:

- a **working** copy that takes `set`, `add` and `remove`;
- a **committed** copy written by `commit`, which the caller saves to its own settings store;
- an **active** copy written by `apply`, which is what the system reads.

`synchronize` discards uncommitted edits. `signature` returns a generation that moves only when a commit changes the committed copy. A callback set with `set_callback` receives `Committed(gen)` and `Applied(gen)` events, and fires only when something changed. Keys are validated as `/`-separated paths (no empty segment, ASCII letters, digits, `-`, `_`, `.`, at most 255 bytes).

Values are `PrefValue::{Bool, Int(i64), Text}`. No floats are stored.

The module is not wired into the shell, the SDK or any daemon. It has no storage, no lock and no authorization step.

## Official API names mirrored

Official pages (DocC index of `systemconfiguration/scpreferences`, landing page `systemconfiguration`):

- `SCPreferencesCreate` (session, `PrefsSession::new`)
- `SCPreferencesAddValue` (`add`, refuses an existing key)
- `SCPreferencesGetValue` (`get`)
- `SCPreferencesSetValue` (`set`)
- `SCPreferencesRemoveValue` (`remove`)
- `SCPreferencesCopyKeyList` (`keys`)
- `SCPreferencesGetSignature` (`signature`)
- `SCPreferencesCommitChanges` (`commit`: "commits changes ... to persistent storage")
- `SCPreferencesApplyChanges` (`apply`: "applied to the active configuration")
- `SCPreferencesSynchronize` (`synchronize`)
- `SCPreferencesCallBack` (the callback type, "used when the preferences have been updated or applied", `set_callback`)

Not mirrored: `SCPreferencesCreateWithAuthorization` (no authorization model), and the storage location of the per-system preferences file.

## REA evidence

- `ev_6a417a13853a14975f992a9df8cee11f5c42df125d49c0463b4b699a247bf062`: rendered official page `https://developer.apple.com/documentation/systemconfiguration/scpreferences` (title "SCPreferences", "The handle to an open preferences session for accessing system configuration preferences.", platforms include macOS and Mac Catalyst).

## Tests

`#[cfg(test)] mod tests` in `pref_session.rs`, seven behaviour tests with hand-worked expected values:

1. `set_and_get_keep_keys_sorted`: three keys come back sorted; replacing one keeps the key count at 3.
2. `invalid_keys_are_refused`: `system`, `/`, `/a//b`, `/a/`, `/a b`, `/a/é` are refused; a key of 256 bytes is refused.
3. `add_refuses_an_existing_key`: a second `add` returns `KeyExists` and the first value stays.
4. `remove_reports_whether_a_value_was_there`: `Ok(true)` then `Ok(false)`.
5. `commit_moves_the_signature_only_on_change`: signature 0, then 1 after the first real commit; a second commit with no change returns false and keeps 1.
6. `apply_makes_only_committed_values_active`: a working value is not active; commit alone does not activate it; apply does, and a second apply returns false.
7. `synchronize_discards_uncommitted_edits`, and `callback_sees_commit_then_apply_in_order` (events `Committed(1), Applied(1)` only, no event for no-op calls).

Build and test were not run, per the task rules. Expected values were checked by hand against the code.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| system-configuration | Preferences session for system configuration | partial | springboard/core/src/pref_session.rs |

Only this row is closed. The rest of theme-10 (Core Data, Disk Arbitration, os, Matter, PermissionKit, CallKit, SwiftData, Synchronization, App Intents, IOBluetooth UI, MetricKit, Foundation Models, Contacts, Security Foundation) is untouched. `partial` is used because there is no storage, authorization or SDK wrapper, and the module is not yet tested on the VM or Midget.

## Known limits

- No file or storage backend; the caller owns persistence.
- No authorization or locking, so any caller that holds the session can change it.
- No SDK API for apps (`springboard/sdk/src/defaults.rs` stays per-app only).
- The index (`mcp__akurai_index__code_search`, repo `bifrost-os`) timed out once during this run. The repo lookups were done with `tools/parity/parity.py` and direct reads.
