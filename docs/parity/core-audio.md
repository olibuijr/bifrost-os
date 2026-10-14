# Core Audio parity

Status: partial (not parity). Built on parity/team-k-core-audio; not yet verified on Midget.

## What was built

Two pure Rust models in the SDK, one file each, registered in `springboard/sdk/src/lib.rs` with one `pub mod` line
each. Neither opens a device, talks to the sound server or changes audio. They describe state and apply the checks
the reference platform applies before it accepts a change.

- `springboard/sdk/src/audio_box.rs` (`audio_box::AudioBox`, `audio_box::BoxRegistry`): the Core Audio box object
  (`AudioHardwareBox`) as a record of an enclosure, its device uids, enabled and acquired state. The registry refuses
  a duplicate box or a device that already belongs to another box, refuses to disable an acquired box, refuses to
  acquire a disabled or already acquired box, and refuses to release a box that is not acquired.
- `springboard/sdk/src/audio_virtual.rs` (`audio_virtual::VirtualDevice`, `VirtualDeviceSet`, `DriverStatus`): the
  description a driver plug-in would publish (uid, name, channel count from 1 to 64, sample rate from the supported
  list). `VirtualDevice::stream_format` gives the 32-bit float format from `bifrost-audio-types`. `VirtualDeviceSet`
  refuses duplicate uids. `DriverStatus::current` reports `NoLoader`, and `can_publish` is false, because no
  user-space loader exists on this platform.

## Official API names mirrored

- `AudioHardwareBox` (named in the matrix row; official page not re-read in this branch): the box as the
  container of devices, with an enabled state and an acquired state. The model follows the reference rules that a
  box in use cannot be disabled and that a box is acquired by one client at a time.
- Audio server driver plug-ins (the matrix row names `AudioHardwarePlugin`; official page not re-read in this branch): the device description a plug-in publishes. The checks mirror the HAL's: a device needs a uid and a name, and its
  sample rate and channel count must be valid.

## Not mirrored

- Loading a plug-in and publishing its devices to the HAL. Nothing in the sound server reads a plug-in bundle.
- Applying box enable or acquire state to hardware. The kernel and the sound server own containers; the registry is
  a model only.
- Real virtual loopback audio. An ALSA loopback or PipeWire node would be needed, and neither is exposed to apps.

## Still open in this framework

- App-facing device and stream API (AudioHardwareDevice, AudioHardwareStream).
- Device property listeners for default device, sample rate and volume.
- Channel layout API for apps (see `springboard/audio/types/src/layout.rs`).

## Tests

Unit tests (`#[test]`) sit in both modules: validation of each field, uniqueness rules, the box state transitions and
the stream format of a virtual device. They have not been run in this branch; the lead builds and checks them.
