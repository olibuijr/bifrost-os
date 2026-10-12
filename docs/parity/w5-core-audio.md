# Wave 5: Core Audio parity (default devices and volume control)

Two pure Rust models close the `os` rows of the Core Audio matrix that had a system-layer capability but no
app-facing API. Both are in `springboard/ui/src` (bifrost-ui) and hold state only. Neither opens a device, touches
the mixer or scales a sample. The shell's volume and route code (`springboard/src/media/volume.rs`,
`springboard/src/media/route.rs`) is unchanged; wiring these models to it is a later round.

## What was built

- `springboard/ui/src/default_device.rs` (`default_device::Defaults`): a device list with one chosen default per
  role (input, output, sound effects). A chosen default survives the device leaving the list and returns when the
  device is listed again. While it is away, or when nothing was chosen, the default is the first listed device that
  may take the role. Hidden devices never take a role, and a device can be limited to some roles.
- `springboard/ui/src/volume_control.rs` (`volume_control::VolumeControl`): a level from 0.0 to 1.0 and a mute flag
  kept apart. Muting keeps the level, unmuting restores it, and a level set while muted stays muted. The effective
  gain is 0 while muted. Non-finite values are refused; out-of-range levels are clamped.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each new module.

## Official API names mirrored

Official pages, read through `python3 -I tools/parity/docc.py`:

- `AudioHardwareSystem` (https://developer.apple.com/documentation/coreaudio/audiohardwaresystem): the process default
  devices `defaultOutputDevice`, `defaultInputDevice`, `defaultSoundEffectsDevice`, each "nil if there is no default".
  `Defaults::default_for` returns None under the same condition.
- `AudioHardwareDevice` (https://developer.apple.com/documentation/coreaudio/audiohardwaredevice): the
  `canBeDefaultInputDevice`, `canBeDefaultOutputDevice`, `canBeDefaultSoundEffectsDevice` flags and `isHidden`
  ("cannot be the default device"). `Device::can_take` implements them.
- `AudioHardwareControl` (https://developer.apple.com/documentation/coreaudio/audiohardwarecontrol): the
  `volumeScalarValue` property (range 0.0 to 1.0 inclusive) and the separate mute control described as "gain, mute".
  `VolumeControl::level` and `set_level` follow the scalar range. The mute flag is kept apart from the level.

Not mirrored: the dB conversion (`volumeDecibelValue`, `convertToDecibels`), which depends on each device's curve, and
the `isProcessOutputMuted` per-process flag. These need device data the system layer does not give apps.

## REA evidence

- `ev_b5cc01c07250210b8d438e7dab744d46ecd148d61c0ede155b528ca67fe4ffac`: Core Audio landing page.
- `ev_6f78279431c13e49eb2da97673a6343a81be97fbd40eef7170dc7514cc9be365`: AudioHardwareSystem.
- `ev_24306f059180e5128ffee1c64fecb23592a0e9063ae71893b1d3ed692af0679f`: AudioHardwareControl.

## Matrix rows closed

| framework slug | feature (as written in the matrix) | new status | BifrOSt path |
|---|---|---|---|
| core-audio | Volume and mute controls | partial | springboard/ui/src/volume_control.rs |
| core-audio | Output routing (speaker or headphones) | partial | springboard/ui/src/default_device.rs |

## Still open

- Neither model is wired into the shell, the audio service or the SDK yet. Nothing is tested on the VM or Midget.
- Apps still cannot choose an output device. The sound server does the routing, and the default-device model does
  not change it.
- Hardware volume keys still go through `springboard/src/media/volume.rs`, which keeps its own mute-as-zero rule.
