# Privacy and permission sheets

Code: `springboard/src/shell/permission_sheet.rs`, `services/privacy_permissions.rs`, `auth_sheet.rs`,
`services/local_auth.rs`, `account_access_sheet.rs`, `contactpick.rs`, `guardian_sheet.rs`, `services/permission_kit.rs`,
`volume_sheet.rs`. Each sheet is an alert card that takes every touch until the person answers.

All sheets default to refusal: an unanswered question never grants.

## Capture permissions (`permission_sheet.rs`)

- An app asks for location, the camera, the microphone or screen recording through the `privacy` service. The shell asks
  Allow or Don't Allow.
- The answer is written with `privacy_permissions::decide`, the same store the Settings switches use.
- Grants are saved to `privacy-grants.tsv` beside the other shell files, including the contacts answers.

## Password (`auth_sheet.rs`, `services/local_auth.rs`)

- An app calling `evaluate` for device-owner authentication gets a pending reply. The shell draws the sheet with the
  app's reason and one secure field.
- The password is checked through the login backend (the same PAM path as the lock screen). It never reaches the app.
- A wrong password keeps the sheet open for another try, up to `SHEET_ATTEMPTS`. A lockout or Cancel ends it, and the app
  gets its answer on the next call.
- Biometrics are always reported as not available.

## Account access (`account_access_sheet.rs`)

- An app's `accounts` request_access call queues a question. The sheet offers Allow, then Don't Allow (the default).
- The answer goes back through the service and is stored with `bifrost_ui::account_store::set_access`.

## Contacts picker (`contactpick.rs`)

- An app starts a pick through the contacts service. The picker sheet lists the contacts; the person selects one
  contact or one value of it.
- Only the final choice is kept for the app. A new pick replaces an unread earlier one. The app reads the result once,
  and reading clears it.

## Guardian (`guardian_sheet.rs`, `services/permission_kit.rs`)

- An app sends a PermissionKit question with `ask`. Only the `significant-app-update` topic is accepted.
- The sheet shows one button per choice, in the order the app gave them, and sends the answer back with `respond`.
- A question nobody answers is dropped when it expires, and the sheet goes with it.

## Encrypted volume unlock (`volume_sheet.rs`)

- After login, the shell lists LUKS volumes that are still closed (`bifrost_core::volume_unlock`) and asks for each
  passphrase in turn.
- Continue passes the passphrase to privd (`disk.unlock_volume`). A failed unlock queues the volume again.
- Cancel leaves the volume locked for this session. The passphrase is never stored.
