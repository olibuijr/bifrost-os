# Wire 5: Bluetooth Settings page (scan row)

Branch `wire5/bluetooth` on top of `main` (release 0.5.42). This round is a small wiring pass over the Bluetooth
pane. The service channel and the SDK client were already wired by `wire-svc-bluetooth.md` and `wire2-bluetooth-net.md`,
and the Settings pane already offers the switch, My Devices, Other Devices, Connect, Disconnect and Forget. No wire
protocol change (`springboard/proto` untouched), no new crate, std only, no unsafe. Nothing was built or run (the lead
builds once).

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/iobluetooth-ui.json`) | BifrOSt path |
|---|---|---|
| iobluetooth-ui | Bluetooth power, scan and device state | `springboard/src/shell/pages.rs` (`bluetooth_spec`): a "Scan for Devices" row under OTHER DEVICES, shown when the radio is on and no scan is running, sends `Cmd::Scan` |

- The scan row sits after the nearby devices, so the first nearby row is still the first device (existing tests keep
  their indexes). While a scan runs the row is hidden and the page shows "Searching…" as before.
- The page scans when it opens (unchanged). The new row lets the user repeat the search without leaving the page.

## How a user reaches it

Settings › Bluetooth (the switch must be On). Under OTHER DEVICES, tap "Scan for Devices" once the search has ended.

## How to verify on Midget (test PC)

1. `bifrost pc midget state`, then open Settings › Bluetooth with the radio On.
2. Wait for "Searching…" to end. The last row of OTHER DEVICES reads "Scan for Devices" (blue).
3. Tap it with `bifrost pc midget tap-window Settings X Y`. The page should show "Searching…" again and the radio
   should report `scanning` in `bifrost pc control` output for the `bluetooth` service `status` reply.
4. Host tests: `bluetooth_page_offers_a_scan_row_only_when_not_scanning` in `springboard/src/shell/pages.rs`.

## REA evidence

None new in this round. The existing evidence for the inquiry and device APIs is in `docs/parity/wire-svc-bluetooth.md`
(`ev_13968174…` for IOBluetoothDevice, `ev_e02c7417…` for IOBluetoothUserNotification).

## Remaining gaps

- Battery level: the shell's Bluetooth `Device` has no battery field and no battery query exists, so no level can be
  shown (`game-controller` battery row stays partial for the same reason).
- Device class and icons: the wire `Device` carries only `kind` (BR/EDR or LE) and `audio`; no class-of-device, so no
  class icon. `bifrost_devices::RemoteDevice::class` stays empty for shell devices.
- Pairing with a user-entered PIN or passkey is not in the page. Only the existing pairing paths are used.
- `bluetooth_page.rs` was not split out: the Bluetooth spec stays in `pages.rs`, because the page is small and the
  shared spec module is where the tests index it.
- Apps have no inquiry UI. `bifrost_ui::bt_chooser` (device selector) is still not used by any sheet.

## Matrix rows

| Slug | Feature (exact) | BifrOSt path |
|---|---|---|
| iobluetooth-ui | Bluetooth power, scan and device state (framework enumerations, constants, functions, BluetoothKeyboardReturnType) | springboard/src/shell/pages.rs (bluetooth_spec, Scan for Devices row) |
| core-bluetooth | Device list and Settings UI | springboard/src/shell/pages.rs:322 (bluetooth_spec) |
