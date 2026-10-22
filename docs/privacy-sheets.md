# Privacy and permission sheets

Code: `springboard/src/shell/permission_sheet.rs`, `services/privacy_permissions.rs`, `auth_sheet.rs`,
`services/local_auth.rs`, `account_access_sheet.rs`, `contactpick.rs`, `guardian_sheet.rs`, `services/permission_kit.rs`,
`volume_sheet.rs`, `usb_sheet.rs`, `network_auth_sheet.rs` with `services/network_auth.rs`, `package_install_sheet.rs`.
Each sheet is an alert card that takes every touch until the person answers.

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

## USB accessories (`usb_sheet.rs`, `bifrost_core::usb_approval`)

- The shell reads `/sys/bus/usb/devices` once a second. A device on a port it has not handled goes through
  `Approvals::arrive_with`: a stored denial blocks it, a stored allowance or a kernel-authorized device is allowed, and an
  unknown device the kernel holds blocked gets the sheet (Always Allow, Allow Once, Don't Allow) and stays blocked until
  the answer. The sheet waits while the screen is locked and goes with a device that is unplugged.
- The `authorized` attribute is written by privd (`usb.authorize`); the shell skips a write that changes nothing.
  "Always" answers are kept in `usb_approvals.txt` in the settings folder, "Allow Once" until the shell restarts.
- On a stock kernel every device is authorized at once, so the sheet meets only devices the system holds back, for
  example with `usbcore.authorized_default=2` (authorize internal devices only). privd sets that policy at run time:
  `usb.policy_set {mode: ask|allow}` writes `/sys/module/usbcore/parameters/authorized_default` (`usbcore` is built in, so
  it is a runtime parameter, not a module option) and saves the word in `/var/lib/springboard/usb_policy`, which privd
  re-applies at start (`usb_approval::apply_saved_policy`); `usb.policy_get` reads it back. There is no Settings switch for
  it yet, and devices already plugged in keep their state.

## Network logins (`network_auth_sheet.rs`, `services/network_auth.rs`, `net_auth.rs`)

- An app with the `network` permission calls the `network_auth` service's `request_login` for an SMB share
  (`kind: "share", host, share`) or an 802.1X network (`kind: "wifi", ssid`). A saved login in the keychain group
  `is.bifrost.network-auth` is returned at once; otherwise the reply is `pending` and the shell asks: user name (starting
  from a refused name), password in a secure field, then whether to remember the login. Cancel or Escape ends it.
- `refused: true` forgets the saved login and makes the sheet say it was not accepted. The typed login goes back to the
  app once; it is saved only when the person chose Remember.

## Package install (`package_install_sheet.rs`, `pacman_update::Source`)

- Opening a package archive (`name-version-release-arch.pkg.tar.zst`) from Files (the workspace `OpenFile` route), or a
  store link `bifrost-package:install/name,name`, opens the sheet. Install runs `pacman -U` or `pacman -S --needed`
  through privd (`package.install_file`, `package.install_repo`) on a worker thread; the sheet follows the pacman log for
  its progress and then shows the result. Return never installs: only the Install button does.
- Installing needs the administrator's password. The confirmation names the package, its version and, for a package
  file, that it is unsigned (`Source::describe`). Install opens the password sheet (the one-field prompt of
  `auth_sheet.rs`, `forms::Purpose::InstallAuthentication`); the password goes to privd's `auth.elevate`, which answers
  with a single-use grant, and only then does the install request go out carrying that grant. privd refuses
  `package.install_file` and `package.install_repo` without one, so the shell alone cannot install (see
  `privd-protocol.md`, "Elevation"). Cancel, Escape and three wrong passwords end the sheet with nothing installed and
  no install request sent. A root shell with no privd has no password to check and installs after the confirmation.

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
