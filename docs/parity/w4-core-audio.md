# Wave 4: Core Audio parity (pure models)

Two gap rows of the Core Audio framework are built as pure models in `bifrost_ui`. Neither module opens a device,
starts a stream or talks to the audio service. The app-facing SDK API for them is not built yet.

## What was built

- `springboard/ui/src/channel_layout.rs`: a channel layout model. It has the standard tags (mono, stereo,
  quadraphonic, 5.1, 7.1), a custom layout from one label per channel (up to 64), the label of each channel,
  a lookup from label to channel index, and a reset to unknown labels. A custom layout whose labels match a standard
  arrangement takes that tag. Errors are typed (`LayoutError`: empty, too many channels, needs labels).
- `springboard/ui/src/audio_clock.rs`: a sample clock model. It has a nominal rate (default 48 kHz) with accepted rate
  ranges, a start that anchors sample position zero to a host time in nanoseconds, conversion in both directions
  between sample position and host time, a stop, a clock domain with a check for hardware sync (same domain, both
  alive), and `suggested_reference`, which picks the first alive running clock. The rate cannot change while the
  clock runs. Errors are typed (`ClockError`).

One `pub mod` line for each module is in `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `ManagedAudioChannelLayout` (tag, number of channels, channel descriptions, initializers from tag and from
  channel descriptions, `setAllToUnknown()`). Doc: https://developer.apple.com/documentation/coreaudio/managedaudiochannellayout
- `AudioHardwareClock` (nominal sample rate, available nominal sample rates, clock domain, is alive, is running,
  `uid`, `setNominalSampleRate`, input and output latency). Doc: https://developer.apple.com/documentation/coreaudio/audiohardwareclock

Not mirrored: the channel bitmap, the `AudioHardwareError` codes, the latency and transport properties, and the
`AudioObjectID` initializer. The device-level data types (`AudioStreamBasicDescription`, `AudioTimeStamp`) are
not in this wave.

## REA evidence

- `ev_9360a19e8a584da69f5e6011018a90ed0532338cd76f6c19d96fd8f79834c6e4`: ManagedAudioChannelLayout page, rendered
  properties and initializers.
- `ev_a7dc008901aeac9f9a6f3669d92753d5538e4ae7a050a73d7465aa8b233c651b`: AudioHardwareClock page, rendered
  properties and `setNominalSampleRate`.

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| core-audio | Channel layouts and data types | partial | springboard/ui/src/channel_layout.rs |
| core-audio | Hardware clocks and sample-accurate sync | partial | springboard/ui/src/audio_clock.rs |

Still open in these rows: no app-facing SDK API for either model, no use from the audio service or the shell,
no data types beyond the channel layout, and no hardware test on the VM or Midget. The sync rule (same domain and
alive) follows the official description. The suggested-reference choice is this model's own rule, not Apple's.

## Tests

`cargo` was not run in this wave, per the rules. Each module has a `#[cfg(test)] mod tests` block with hand-worked
behaviour tests (channel layout: 7; clock: 7).
