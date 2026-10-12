# Wire 6: Bluetooth device type and sound output (Settings › Bluetooth)

Branch `wire6/bluetooth-devices` on top of `main` (release 0.5.43). Small wiring pass over the Bluetooth detail page
and the shell's device table. No wire protocol change (`springboard/proto` untouched), no new crate, std plus the
existing `bifrost-core` dependency, no unsafe. Nothing was built or run (the lead builds once).

## What is now wired

| Matrix slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| iobluetooth | Remote device model (name, class, paired, connected) | `springboard/src/bare/bluetooth.rs` (`Device.class`, `Device::type_label`), `springboard/src/shell/pages.rs` (BluetoothDetail "Type" row), `springboard/src/shell/services/bluetooth_service.rs` (`class`, `type` in device JSON) |
| iobluetooth | Audio output routing to a headset | `springboard/src/shell/pages.rs` (BluetoothDetail "Use for Sound Output"), `springboard/src/shell/settings.rs` (`Action::AudioOutput`), `springboard/src/shell/mod.rs` (`Effect::BluetoothOutput`, `Shell::use_bluetooth_output`), `springboard/src/bin/shell.rs` (effect arm), `springboard/src/shell/services/audio_devices.rs` (test) |
| iobluetooth-ui | Bluetooth power, scan and device state (framework enumerations, constants, functions, BluetoothKeyboardReturnType) | `springboard/src/bare/bluetooth.rs` (`Device.class`, `type_label`) with the Settings detail page |

- The class of device comes from the inquiry's EIR data (the same parse that already read the name and the audio
  profiles). `fill` keeps the last class heard. Bare `Device` gained `class: Option<u32>`; `None` means no class yet
  (an LE device or one the scan has not described).
- `type_label` maps the class through `bifrost_core::bt_address::ClassOfDevice::kind()`: Headphones, Headset,
  Hands-Free, Speaker, Microphone, Keyboard, Pointing Device, Keyboard and Pointing Device, Computer, Phone, Device,
  or Unknown.
- "Use for Sound Output" appears on a device that is connected, is an audio device, and is not busy. Tapping it
  sends `set_default` `{role: output, id: bluetooth}` through the `audio-devices` service. That call succeeds only
  while the headset is connected (the service lists the Bluetooth output only then), and the sound server routes
  samples as before.
- The Bluetooth service's device JSON gained `class` (number or null) and `type` (label). The existing keys are
  unchanged.

## How a user reaches it

Settings › Bluetooth (radio On). Open a device under MY DEVICES. The "Type" row names the device's kind. When the
device is connected and is an audio device, the first group offers "Use for Sound Output" under Disconnect.

## How to verify on Midget (test PC)

1. Pair and connect a Bluetooth headset or speaker (Settings › Bluetooth, tap the device under OTHER DEVICES).
2. Open the device under MY DEVICES: the "Type" row reads "Headphones", "Hands-Free" or "Speaker" (not "Unknown")
   once the scan has heard its class. A device paired in an earlier session reads "Unknown" until it is seen again.
3. Tap "Use for Sound Output". Then `bifrost pc control` on the `audio-devices` service, `list`: the `defaults.output`
   value is `bluetooth`. A failure is printed to the shell log as `springboard: sound output: ...`.
4. Host tests: `a_device_is_labelled_by_its_class_of_device` (bare/bluetooth.rs),
   `bluetooth_detail_offers_connect_or_disconnect_and_forget` (shell/pages.rs),
   `a_connected_bluetooth_audio_device_takes_the_output_default` (shell/services/audio_devices.rs).

## REA evidence

No new REA capture in this round. The device model evidence is the existing set in `docs/parity/wire-svc-bluetooth.md`:
`ev_13968174b902e40c51404431d77aedd9647507c73e949ff68ef5e3e1bfca2e8e` (IOBluetoothDevice) and
`ev_e02c7417062086993a909f42ce9dca559e26d7e4edbc4beabf243bcbe6b6f381` (IOBluetoothUserNotification).

## Remaining gaps

- Battery level: not shown. BlueZ `Battery1` lives on the system D-Bus, and the bare layer reads only the kernel
  Bluetooth management socket (`mgmt`), which has no battery command. Reading it needs a D-Bus client, which is a new
  dependency this round does not add. `game-controller` battery stays partial for the same reason.
- Device class icons: the kind is shown as text ("Type" row), not as a drawn icon. A drawn glyph needs artwork in
  `shell/row_glyphs.rs` (or new `Icon` tiles), which is a separate design pass.
- The class is not stored in `bluetooth.ini`. A paired device reads "Unknown" until a scan hears it again.
- Selecting the output is not undone when the headset disconnects. The default output then falls back to the
  service's normal rules (the speaker), which is how the service already handles a removed device.
- Device selector (`bifrost_ui::bt_chooser`) is still not used by any sheet; no app inquiry UI.
- RSSI and advertisement data for core-bluetooth are not shown on the Settings page in this round.

## Matrix rows

| Slug | Feature (exact) | BifrOSt path |
|---|---|---|
| iobluetooth | Remote device model (name, class, paired, connected) | springboard/src/bare/bluetooth.rs (Device.class, type_label); springboard/src/shell/pages.rs (BluetoothDetail, Type row) |
| iobluetooth | Audio output routing to a headset | springboard/src/shell/pages.rs (Use for Sound Output); springboard/src/shell/mod.rs (use_bluetooth_output); springboard/src/shell/services/audio_devices.rs |
| iobluetooth-ui | Bluetooth power, scan and device state (framework enumerations, constants, functions, BluetoothKeyboardReturnType) | springboard/src/bare/bluetooth.rs (Device.class, type_label) |
