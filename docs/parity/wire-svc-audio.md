# Wire: audio devices service (`audio-devices`)

Wires the Core Audio device and default-device models into the desktop as a shell system service, so apps can list
the output and input devices, choose the default for each role, read and set the output volume and mute, and hear
about route changes. It uses the existing service channel (`ToShell::ServiceCall` / `ToApp::ServiceEvent`); the wire
protocol is unchanged.

## What is now wired

| Model (springboard/ui/src) | Used for |
|---|---|
| `default_device::Defaults` | The device list and the chosen default per role (input, output, sound effects). A chosen default survives the device leaving and returns when it comes back. |
| `default_device::Device` | One device with its roles and hidden flag. Built from the shell's route, Bluetooth and capture state. |
| `volume_control::VolumeControl` | The output level and mute kept apart. `set_volume` clamps through it; `volume` reports `level`, `muted`, `gain`. |

Shell side: `springboard/src/shell/services/audio_devices.rs` (`AudioDevices`, service name `audio-devices`), registered
in `services::all()`. SDK side: `springboard/sdk/src/audio_devices.rs` (typed client and reply and event decoders),
exported as `bifrost_app::audio_devices`.

### Devices the shell reports

- `headphones` (Headphones): listed only while the headset jack is in (`cc.route == Route::Headphones`).
- `bluetooth` (the connected Bluetooth audio device's name): listed when `info.bt.audio_sink()` is set.
- `speaker` (Speaker): always listed.
- `microphone` (Microphone, input only): listed when `/proc/asound/pcm` has a capture device. Probed at most every 2 s.

List order is headphones, Bluetooth, speaker, microphone. The automatic default for output is the first listed device
that can take it, so the headset wins over the speaker while it is plugged in.

### Methods (shell service `audio-devices`)

| Method | Params | Reply |
|---|---|---|
| `list` | none | `{devices: [{id, name, input, output, sound_effects}], defaults: {input, output, sound_effects}}` |
| `set_default` | `{role, id}` | `null`; errors: unknown role, no such device, device cannot take the role |
| `clear_default` | `{role}` | `null` |
| `volume` | none | `{level, muted, gain}` (gain is 0 while muted) |
| `set_volume` | `{level}` (0 to 1) | the `volume` reply |
| `set_muted` | `{muted}` | the `volume` reply |

`role` is `input`, `output` or `sound_effects`.

### Events (to every running app)

- `devices_changed` `{devices: [id]}` when the device list changes.
- `route_changed` `{route, output}` when the output route (Speaker or Headphones) or the default output changes.

Events are produced in the service `tick`. The first tick only records state, so nothing is sent at start-up.

### SDK client

`bifrost_app::audio_devices`: `list`, `set_default`, `clear_default`, `volume`, `set_volume`, `set_muted` (each returns
the request id), `decode_list`, `decode_volume`, `decode_event` (returns `Change::Route` or `Change::Devices`), and
`role_name`. Apps read replies in `App::service_reply` and events in `App::service_event`.

## How a user reaches it

Apps reach it. There is no new Settings page or menu item in this round. The Control Center volume slider and the
mute tile already drive the same state, and `volume` / `set_volume` / `set_muted` change the same level the slider shows.

## How to verify on the test PC (Midget)

1. Update Midget to the release that contains this change (`bifrost pc midget ota`, until `phase=accepted`).
2. Confirm the service is registered: call `system` `services` through an app, or `bifrost pc midget state`; the list
   includes `audio-devices`.
3. From a test app (or `bifrost pc control call` on an app that uses the SDK client), call `audio-devices` `list`.
   With nothing plugged in the reply has `speaker` as the output default.
4. Plug in headphones: `route_changed` arrives with `route: "Headphones"` and `output: "headphones"`, and `list` now
   shows `headphones` first. Unplug: the route returns to `Speaker`.
5. Call `set_volume` with 0.3 and check the Control Center slider moved. Call `set_muted` with `true`: `volume` returns
   `gain: 0` and the level stays 0.3.
6. Call `set_default` with `{role: "output", id: "speaker"}` while headphones are in: after unplugging, the speaker is
   still the default. Call `set_default` with `microphone` for `output`: it must be refused.

## Tests

- `springboard/src/shell/services/audio_devices.rs`: device presence and order, microphone input-only, sync that keeps
  a chosen default while the device is away, headset as automatic output, microphone never output, role names, volume
  view (gain 0 while muted), event diff (one event per change, none on the first look), list reply, device errors.
- `springboard/sdk/src/audio_devices.rs`: list and volume reply decoding, missing-field errors, route and device event
  decoding, role names.

Not run in this round (no builds in this worktree; the lead builds once).

## REA evidence

Official pages read with `python3 -I tools/parity/docc.py`: `coreaudio/audiohardwaredevice` (the
`canBeDefaultInputDevice`, `canBeDefaultOutputDevice`, `canBeDefaultSoundEffectsDevice` and `isHidden` flags) and
`coreaudio/audiohardwaresystem` (`defaultInputDevice`, `defaultOutputDevice`, `defaultSoundEffectsDevice`, "nil if
there is no default").

Rendered page captures (`tools/parity/rea-page.sh`):

- `ev_93cd1485507b2725b829b6e743dd267709255496d06e919f593b501523b1a1ec`: AudioHardwareDevice.
- `ev_f573b51d47a70746544d7837da7ab84e0de746bf2f2ec18c21edee3396b75f73`: AudioHardwareSystem.

Earlier evidence the models were built from (wave 5 note, `docs/parity/w5-core-audio.md`):
`ev_6f78279431c13e49eb2da97673a6343a81be97fbd40eef7170dc7514cc9be365` (AudioHardwareSystem),
`ev_24306f059180e5128ffee1c64fecb23592a0e9063ae71893b1d3ed692af0679f` (AudioHardwareControl).

## Matrix rows this wiring moves

| framework slug | feature (as written in the matrix) | BifrOSt path |
|---|---|---|
| core-audio | Output routing (speaker or headphones) | springboard/src/shell/services/audio_devices.rs, springboard/ui/src/default_device.rs |
| core-audio | Volume and mute controls | springboard/src/shell/services/audio_devices.rs, springboard/ui/src/volume_control.rs |
| core-audio | Playback transport and now-playing state (app client) | unchanged (the `audio` service) |

The matrix itself is not edited here (`docs/parity-matrix/` is owned by the lead).

## Remaining gaps

- Apps cannot change where the sound is routed. The service changes the default-device choice and the route state the
  shell reports, but the sound server still does the routing; the shell's `set_route` is unchanged.
- No Bluetooth device identifier: the Bluetooth entry is keyed `bluetooth`, so two Bluetooth outputs are not told apart.
- Input has one entry (the built-in microphone from the capture probe). No input device selection and no input volume.
- `set_volume` and `set_muted` change the Control Center level and the music mute the shell already keeps; per-device
  volume (`VolumeControl` per device) is not kept.
- The `isProcessOutputMuted` and dB conversion (`volumeDecibelValue`) are not exposed (see `w5-core-audio.md`).
- Aggregate devices, process taps and the HAL plug-in loader remain out of scope (`w4-core-audio.md`, `w5-core-audio.md`).
- `audiod` (the `is.olibuijr.audio` service) does not expose devices; this service reads the shell's state, not audiod's.
