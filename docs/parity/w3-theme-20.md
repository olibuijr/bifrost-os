# Wave 3, theme 20: process taps (capture a process's output)

## What was built

`springboard/ui/src/process_tap.rs` (`bifrost_ui::process_tap`), a pure model with no device access:

- `TapDescription` names the tapped processes (`of_processes`), or every process except the listed ones
  (`excluding`, an empty list taps everything). It has a mixdown (`Source`, `Mono`, `Stereo`), a mute behaviour
  (`Unmuted`, `Muted`, `MutedWhenTapped`), a private flag and a name. `validate` checks the description against a
  caller-supplied process test, so it does not depend on the process list in `audio_hardware.rs`.
- `convert_frames` converts interleaved f32 frames between channel counts: copy, mono fan-out, mean-down
  (channel index modulo the output count), and silence fill.
- `FrameRing` is a bounded frame queue. When full it drops the oldest frames and counts them as overruns.
- `ProcessTap` checks, starts and stops a tap, converts pushed source frames to the tap's format while running,
  reports whether its output is muted, and returns `AudioBlock`s tagged `CaptureKind::AppAudio` or
  `CaptureKind::Microphone`.

The module is registered with one `pub mod process_tap;` line in `springboard/ui/src/lib.rs`. It has 8 tests in
`#[cfg(test)] mod tests`. Nothing was built or run, as the task required.

## Official API names mirrored

- Core Audio: `CATapDescription` (initializers `init(processes:deviceUID:stream:)`,
  `init(excludingProcesses:deviceUID:stream:)`, `init(monoMixdownOfProcesses:)`, `init(stereoMixdownOfProcesses:)`,
  `init(monoGlobalTapButExcludeProcesses:)`, `init(stereoGlobalTapButExcludeProcesses:)`; properties `isExclusive`,
  `isMixdown`, `isMono`, `isPrivate`, `muteBehavior`, `processes`, `name`), and `CATapMuteBehavior`
  (`unmuted`, `muted`, `mutedWhenTapped`). Sources: https://developer.apple.com/documentation/coreaudio/catapdescription
  and https://developer.apple.com/documentation/coreaudio/catapmutebehavior
- ReplayKit: `RPSampleBufferType` audio cases `audioApp` and `audioMic`. The official page marks this type
  deprecated in favour of `SCStreamOutputType`, so `CaptureKind` mirrors only the two audio kinds and no video.
  Source: https://developer.apple.com/documentation/replaykit/rpsamplebuffertype

## REA evidence

- Core Audio, `CATapDescription` page: `ev_0688f7c527e61a2409e11c96fc7533d9adfe9a4c7f86233131bad368b578b281`
- ReplayKit, `RPSampleBufferType` page: `ev_ffee3684bea561a9703c785ffb3f47937d4b01ea5c0ce2905cdd5623beb30212`

## Gaps that remain

- The tap is not connected to `springboard/audiod`. The audio service has no output-side hook, so no real process
  audio reaches `ProcessTap::push` yet.
- No process ids are checked against a live process list. `validate` takes a caller-supplied test.
- Mixdown is a plain channel mean. It is not a channel layout with per-channel labels.
- No UUID, aggregate-device or device-UID binding (the `deviceUID` and `stream` parameters of the official
  initializers) and no restore-on-relaunch flag.
- `Media Toolbox` audio processing taps (`MTAudioProcessingTap`) are not modelled. Their prepare, process and
  storage callbacks are a different surface and are left for a later theme.
- ReplayKit's `startCapture` handler and its `CMSampleBuffer` delivery are not modelled. `AudioBlock` is the audio
  half only.

## Matrix rows closed

| Framework slug | Feature (as in data) | New status | BifrOSt path |
|---|---|---|---|
| core-audio | Process taps (capture a process's output) | partial | springboard/ui/src/process_tap.rs |
| replaykit | App and system audio capture | partial | springboard/ui/src/process_tap.rs |

## Summary

A pure process-tap model (description, mixdown, mute behaviour, format conversion, bounded frame ring and tap
lifecycle) now exists in bifrost-ui, mirroring the official tap description and mute-behaviour names. It closes the
two audio-capture rows as partial because nothing feeds it from the audio service yet.
