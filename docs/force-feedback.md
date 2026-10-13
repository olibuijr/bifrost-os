# Force Feedback (parity notes)

Reference framework: Apple's Force Feedback (legacy C API for wheels and joysticks, macOS 10.2+). Matrix data:
`docs/parity-matrix/data/force-feedback.json`.

## What exists

- `springboard/core/src/force_effect.rs`: pure device model. Effect kinds (constant, periodic, ramp, condition),
  envelope, a 16-slot device with gain and autocenter, and the output level at a time. It sends nothing to the kernel.
- `springboard/core/src/gamepad.rs`: `supports_rumble` reads the rumble bit of the force-feedback bitmap.
- `springboard/core/src/rumble.rs`, `ff_device.rs`, `ff_effect.rs`: rumble and device-level models that other rows use.

## Decisions in this round

- **App-facing haptic request.** The SDK's parameterless `haptic()` maps to `Pattern::Generic`. Three patterns
  (generic, alignment, level change) follow the reference haptic API's feedback patterns. The levels (0.6, 0.4, 0.8)
  and durations (40, 20, 60 ms) are BifrOSt's own choice, not values measured from Apple hardware.
  `Device::pulse` plays one pulse and `Device::reap` frees finished pulses, so a stream of pulses does not fill the
  16 slots. Effects created by the app are never reaped.
- **Capability detection.** `has_force_feedback` tests the `EV_FF` bit of the event-type bitmap; `supports_constant_force`
  tests `FF_CONSTANT` in the force-feedback bitmap. Every pulse is a constant force, so a node without this bit cannot
  play one.
- **Not done.** Hardware escape commands (driver pass-through) and cooperative levels are not implemented. The
  cooperative level is unimplemented in the reference framework itself.

## Still open

- The shell does not yet hold a `Device` or upload effects with `EVIOCSFF`. `SDK Runtime::haptic()` (`sdk/src/runtime.rs`) sends `ToShell::Haptic`, and the shell
  has no handler for it, so it is dropped (checked 2026-10-13). Wiring needs the shell to own a device per controller and write the
  kernel effect structs; that is a follow-up.
- Nothing is tested on the VM or on Midget. Until it is, the rows stay `partial`.
