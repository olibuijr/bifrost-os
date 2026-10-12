# Wave 5, theme 05: media reading and writing (force-feedback slice)

## What was built

`springboard/core/src/force_effect.rs` (registered in `springboard/core/src/lib.rs`): a pure, std-only model of
force-feedback effects for a wheel or joystick.

- Effect kinds: `Shape::Constant`, `Shape::Periodic` (square, triangle, sine, saw up, saw down, with offset and phase),
  `Shape::Ramp`, and `Shape::Condition` (spring against displacement, damper against velocity, with deadband,
  centre, coefficients and saturation).
- `Envelope`: attack and fade with their levels; `delay_ms`, `duration_ms` and `INFINITE_MS` as in the rumble model.
- `Device`: up to `MAX_EFFECTS` (16) effects, each stopped or playing. `create`, `set_parameters`, `start`, `stop`,
  `unload`, `is_playing`; a device gain and an autocenter level; `output` sums the playing effects, scales by gain and
  clamps to -1..1; `output_s16` gives the same force as a signed 16-bit level.
- Errors: `ZeroPeriod`, `ZeroDuration`, `Full`, `UnknownEffect`, with `Display`.
- Tests: 9 behaviour tests in `#[cfg(test)] mod tests`, expected values worked out by hand from the formulas
  (square flips at half period, triangle and sine peaks, ramp midpoint, envelope at 50, 200 and 900 ms, spring
  deadband and saturation, damper on velocity, the 16-effect cap and validation, gain and clamp, start time and stop).

The model drives no device. Uploading the effect to the input node (the evdev effect call), reading the effect
capability bit and feeding the wheel's displacement are left to the caller. Nothing is wired into the shell or an app
SDK, and it is not tested on hardware.

## Apple APIs mirrored

Force Feedback framework (official page: https://developer.apple.com/documentation/forcefeedback, macOS 10.2+,
Mac Catalyst 13.0+):

- `FFDeviceCreateEffect`, `FFEffectDownload` (download is folded into `create`), `FFEffectStart`, `FFEffectStop`,
  `FFEffectUnload`, `FFEffectSetParameters`: `Device::create`, `start`, `stop`, `unload`, `set_parameters`.
- `FFEFFECT` with the `FFPERIODIC`, `FFCONSTANTFORCE`, `FFRAMPFORCE`, `FFCONDITION`, `FFENVELOPE` structures: `Effect`,
  `Shape`, `Envelope`.
- Gain and autocenter properties (`FFDeviceSetForceFeedbackProperty`, `FFProperty`): `Device::set_gain`,
  `set_autocenter`. The property and state query calls (`FFDeviceGetForceFeedbackProperty`,
  `FFDeviceGetForceFeedbackState`) are not mirrored.

## REA evidence

- Force Feedback landing page: `ev_9701cea4c6353b6c8670bd56df7c95da2df375f350eeb27c0cd3e2e2087513d9`
  (rendered via `tools/parity/rea-page.sh`, title "Force Feedback | Apple Developer Documentation").
- Symbol pages were read through `python3 -I tools/parity/docc.py forcefeedback/ffeffect`; no separate REA id was
  recorded for them.

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| force-feedback | Effect creation, download and playback | partial | springboard/core/src/force_effect.rs |
| force-feedback | Device properties and force-feedback state | partial | springboard/core/src/force_effect.rs |

Remaining in this theme (not built here): the other rows of the theme, including media reader and writer, Core Motion
raw sensor events, Video Toolbox encoding, Dispatch I/O channels and the Matter and AVKit items.
