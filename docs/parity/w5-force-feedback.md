# Force Feedback (parity wave 5)

Official reference: https://developer.apple.com/documentation/forcefeedback (the framework is a legacy C API for
force-feedback devices such as wheels and joysticks; the landing page says it is for controlling those devices and for
writing plug-ins).

REA evidence of the rendered page:

- Framework landing page: `ev_9f5b80befb25940153ed235d9a19cafca104259c9f69c8987a6d45d4df819e96`

Official symbol pages read with `docc.py`: `ffeffect`, `ffconstantforce`, `ffperiodic`, `fframpforce`, `ffcondition`,
`ffenvelope`, `ffcapabilities`. The `ffstate` and `ffproperty` pages have no abstract, so the property and state names
below come from the matrix row, not from those pages.

Waves 1 to 4 added the device capability model (`springboard/core/src/gamepad.rs`) and the rumble model
(`springboard/core/src/rumble.rs`). This round closes the two gap rows for effects and device properties. Both are pure
models in `springboard/core` (std only). Neither is wired to an evdev node, the shell or an SDK call, so the matrix
status is `partial`.

## What was built

### `springboard/core/src/ff_effect.rs` (`Effect`, `Kind`, `Envelope`, `EffectError`)

One force-feedback effect, mirroring the effect structure and its type-specific parts.

- `Kind::Constant { magnitude }`, `Kind::Ramp { start, end }`, `Kind::Periodic { magnitude, offset, phase, period_us }`
  and `Kind::Condition { ... }`: the four effect kinds that Apple's constant force, ramp force, periodic and condition
  structures describe.
- `Envelope { attack_level, attack_us, fade_level, fade_us }`, the optional envelope of an effect.
- `Effect::validate` checks each value against its range: gain and levels 0 to 10000, signed magnitudes -10000 to
  10000, phase 0 to 35999, a periodic period above zero, and an envelope whose attack and fade fit inside a finite
  duration.
- `Effect::level_at(t_us)` gives the signed output level for constant and ramp effects, with gain and envelope applied.
  It returns `Some(0)` once the duration has passed, and `None` for periodic and condition effects, which have no level
  curve in this model.

The envelope rule is BifrOSt's own, written in the module comment: linear attack from the attack level to the sustain,
a held sustain, and a linear fade over the last fade time to the fade level. Apple's page describes the envelope fields
but not the interpolation, so the rule is not claimed as Apple's.

Tests: 7 (range checks, periodic phase and period, envelope fit, constant level with gain, ramp, envelope shape, sign
and no-curve case).

### `springboard/core/src/ff_device.rs` (`Device`, `Property`, `Command`, `DeviceError`)

The state of one device: its properties, its commands and sixteen effect slots.

- `Device::download` validates an effect and stores it in the first free slot (`FFDeviceCreateEffect` and
  `FFEffectDownload` in the official naming; the slot index replaces the effect handle). `set_parameters`, `start`,
  `stop` and `unload` work on a slot.
- `Property::Gain` and `Property::Autocenter`, with `property` and `set_property`. Values above 10000 are clamped and
  the stored value is returned (`FFDeviceGetForceFeedbackProperty` and `FFDeviceSetForceFeedbackProperty`).
- `Command` covers reset, stop all, pause, continue, and motors on and off (the device-wide commands of
  `FFDeviceSendForceFeedbackCommand`; the command names are BifrOSt's).
- `Device::output_level(slot, t_us)` gives the level the device should output: zero while paused, with the motors off,
  or before the effect is started; the start delay is waited out; the device gain scales the result.
- `Device::is_playing` reports whether the app started a slot and has not stopped it.

Tests: 6 (download, start, stop and unload on one slot; a full device and an invalid effect that takes no slot; gain
scaling and clamping; pause and motors; start delay; reset and stop all).

## Matrix rows closed

| Framework slug | Feature (as written in `docs/parity-matrix/data/force-feedback.json`) | New status | BifrOSt path |
|---|---|---|---|
| force-feedback | Effect creation, download and playback | partial | `springboard/core/src/ff_effect.rs`, `springboard/core/src/ff_device.rs` |
| force-feedback | Device properties and force-feedback state | partial | `springboard/core/src/ff_device.rs` |

## Not done in this round

- App-facing haptic request: the shell still drops `ToShell::Haptic` (`springboard/src/shell/apphost.rs`). Wiring it
  needs a shell change and a device to receive it, so it is left for a later round.
- Hardware escape commands and the cooperative level: no driver pass-through exists. The matrix row notes that the
  cooperative-level call is marked unimplemented in Apple's version 1.0 reference; that note is not re-checked here.
- The evdev `EV_FF` read path and the upload of effects to a device node: `springboard/src/bare/input.rs` does not read
  force-feedback bits yet, so the models above are not connected to hardware.
