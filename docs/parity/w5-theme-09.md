# Wave 5, theme-09: object push transfer UI

Theme: "Object push transfer UI (IOBluetoothObjectPushUIController)", 16 gap rows across 15 frameworks. This wave
closes three rows honestly with pure models. The other 13 rows need a radio, a network service, a provider
extension or a platform account, so they stay open.

## What was built

| Module | Crate | What it models |
|---|---|---|
| `springboard/ui/src/object_push.rs` | bifrost-ui | One push of files to a device: device, title, icon, file list, state (idle, in progress, finished, stopped), bytes sent, progress, the file being sent. `start` refuses with no device, no files, or a second start. Bytes are clamped to the total. A push of empty files finishes at once. `stop` only cancels a push in progress. |
| `springboard/ui/src/passkey_display.rs` | bifrost-ui | The six-digit pairing passkey with the device and the simple-secure-pairing flag. Passkeys that are not exactly six ASCII digits are refused. The indicator walks across the digits (0 to 6, clamped at both ends) and splits the passkey into typed and remaining digits. |
| `springboard/sdk/src/encryption_preference.rs` | bifrost-sdk | The `optional`, `required` and `none` choices. `negotiate` decides one pair (a required side refuses a none peer). `session_encrypted` decides a session against all its peers. |

Each module has a `#[cfg(test)] mod tests` with hand-worked expectations (6 to 7 tests per module). Each module is
registered with one `pub mod` line in its crate's `lib.rs`.

Not wired into the shell or into apps, and not run (no build in this wave). The transport does not send the bytes,
and the panel is not drawn.

## Official API names mirrored

- `IOBluetoothObjectPushUIController`: `init(objectPushWith:withFiles:delegate:)`, `init(objectPushWithBluetoothDevice:withFiles:delegate:)`, `runModal`, `runPanel`, `beginSheetModal(for:modalDelegate:didEnd:contextInfo:)`, `stop`, `isTransferInProgress`, `getDevice`, `setIconImage(_:)`, `setTitle(_:)`, `getTitle`.
- `IOBluetoothPasskeyDisplay`: `passkey`, `setPasskey(_:for:usingSSP:)`, `advancePasskeyIndicator`, `retreatPasskeyIndicator`, `resetPasskeyIndicator`, `isIncomingRequest`.
- `MCEncryptionPreference`: `optional`, `required`, `none`. The page's rule: a connection uses encryption when all peers choose optional or required. If some peers choose none, the session is not encrypted.

The modal, sheet and panel run calls are left out of the model. They belong to the drawn panel.

## REA evidence

- `ev_055980d27b1bcd7ace21ece5948d4297000f51d2fa4048791f36bf16a97ae080`: IOBluetoothObjectPushUIController, https://developer.apple.com/documentation/iobluetoothui/iobluetoothobjectpushuicontroller
- `ev_36452e0119f292341e0326f9f36743c32d50b1fcf35f0fc3b21e8242764d1395`: IOBluetoothPasskeyDisplay, https://developer.apple.com/documentation/iobluetoothui/iobluetoothpasskeydisplay
- `ev_cbc2e1f4d4df1520661c7feeba92d2f8d1fb0e515bde9f59c2e49e9d6673ef5d`: MCEncryptionPreference, https://developer.apple.com/documentation/multipeerconnectivity/mcencryptionpreference (the page is marked Deprecated, "Use Network Framework instead")

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| iobluetooth-ui | Object push transfer UI (IOBluetoothObjectPushUIController) | partial | springboard/ui/src/object_push.rs |
| iobluetooth-ui | Passkey and confirmation display (IOBluetoothPasskeyDisplay) | partial | springboard/ui/src/passkey_display.rs |
| multipeer-connectivity | Encryption preference (MCEncryptionPreference) | partial | springboard/sdk/src/encryption_preference.rs |

Rows left open in this theme: network (service discovery), mailkit, cloudkit, coreaudiokit, replaykit, avkit,
social, cfnetwork (FTP), file-provider, iousbhost, apple-archive, media-toolbox and system-configuration. Each
needs a provider, a network service or a platform account that BifrOSt does not have.
