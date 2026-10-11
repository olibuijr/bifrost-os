# Theme 10: Audio devices

Pure Rust models in `springboard/ui/src` (bifrost-ui) for the audio objects a sound client sees: the speech
synthesis queue and voice choice, the app audio session with its interruptions, playback arbitration between
participants, and the hardware objects (device boxes, aggregate devices, client processes). None of the modules
opens a device, starts a stream or calls the audio service. They are the state and rules that the audio service
and the SDK will apply.

## What was built

| Module | Change | Mirrors (official names) |
|---|---|---|
| `springboard/ui/src/speech.rs` | Extended: `Utterance::pre_delay` / `with_pre_delay`, `Synthesizer::is_speaking` / `is_paused`, `voices_for_language` (exact matches first, then primary-subtag matches). | AVSpeechUtterance (`preUtteranceDelay`, `postUtteranceDelay`, `voice`), AVSpeechSynthesizer (`isSpeaking`, `isPaused`), AVSpeechSynthesisVoice |
| `springboard/ui/src/audio_session.rs` (new) | Category, mode and options; activation and deactivation; interruptions with the resume recommendation; an ordered event log. | AVAudioSession (`setCategory`, `setMode`, `setActive`, `activate`/`deactivate`, `category`, `mode`, `categoryOptions`), the interruption and activation notifications, `AVAudioSession.InterruptionContext`, `AVAudioSession.ResumptionRecommendation` |
| `springboard/ui/src/playback_arbiter.rs` (new) | Participant registry, preferred participant for external playback and for non-mixable routes, and the `may_play_*` checks. Preferences are cleared when their participant is removed. | AVRoutingPlaybackArbiter (`preferredParticipantForExternalPlayback`, `preferredParticipantForNonMixableAudioRoutes`), AVRoutingPlaybackParticipant |
| `springboard/ui/src/audio_hardware.rs` (new) | `DeviceBox` (enable and disable, devices hidden while disabled, device-type flags, protected flag); `AggregateDevice` (device and tap members with unique uids, clock source limited to members); `AudioProcess` and `ProcessList` (running IO, input and output stream counts, output devices reported only while running output). | AudioHardwareBox (`enabled`, `devices`, `enable()`, `disable()`, `hasAudio`, `hasMIDI`, `hasVideo`, `isProtected`), AudioHardwareAggregateDevice (`subdevices`, `subtaps`, `clockSource`, `setClockSource`), AudioHardwareProcess (`bundleID`, `pid`, `isRunning`, `isRunningInput`, `isRunningOutput`, `devices`) |

`springboard/ui/src/lib.rs` gained three `pub mod` lines, one per new module.

Behaviour that is a BifrOSt model rule and not a documented Apple rule:

- `audio_session`: `SessionError::Interrupted` (activation refused during an interruption) is a model rule. The
  official pages on the session's interruption handling were not read in full for this round, so the exact system
  behaviour is unverified.
- `audio_session`: the meanings written on each `Category` variant (for example "stops other audio while active")
  are short summaries for this model. They are not quoted from the category reference, and the category option
  semantics beyond the four booleans are not modelled.
- `audio_hardware`: a disabled box keeps its device list and returns an empty slice from `devices()`. This follows
  the documented rule that box contents are only available when the box is enabled, and the list is kept here so
  that re-enabling works without a rescan.
- `playback_arbiter`: `shared()` (the singleton) is not modelled. The arbiter is a plain value that the audio
  service owns.

## Official API names mirrored

Checked against the DocC JSON with `tools/parity/docc.py`:

- `avfaudio/avspeechutterance`: `init(string:)`, `voice`, `pitchMultiplier`, `volume`, `rate`, `preUtteranceDelay`,
  `postUtteranceDelay`.
- `avfaudio/avspeechsynthesizer`: `speak(_:)`, `pauseSpeaking(at:)`, `continueSpeaking()`, `stopSpeaking(at:)`,
  `isSpeaking`, `isPaused`, `write(_:toBufferCallback:)`. The buffer write path and the boundary-based pause and
  stop are not implemented.
- `avfaudio/avaudiosession`: `setCategory(_:mode:options:)`, `setMode(_:)`, `setActive(_:options:)`,
  `activate(options:completionHandler:)`, `deactivate(options:completionHandler:)`, `category`, `mode`,
  `categoryOptions`, the activation and interruption notification set.
- `avrouting/avroutingplaybackarbiter`: `preferredParticipantForExternalPlayback`,
  `preferredParticipantForNonMixableAudioRoutes`, `shared()`.
- `coreaudio/audiohardwarebox`: `enabled`, `devices`, `enable()`, `disable()`, `hasAudio`, `hasMIDI`, `hasVideo`,
  `isProtected`, `uid`.
- `coreaudio/audiohardwareaggregatedevice`: `subdevices`, `subtaps`, `clockSource`, `setClockSource(_:)`,
  `setSubdevices(_:)`, `setSubtaps(_:)`.
- `coreaudio/audiohardwareprocess`: `pid`, `bundleID`, `isRunning`, `isRunningInput`, `isRunningOutput`, `devices`.

The `springboard/ui/src` code uses only neutral names (`Session`, `Arbiter`, `DeviceBox`, `AggregateDevice`,
`AudioProcess`, `ProcessList`). The Apple names appear in this document only, as required by the owner rule.

## REA evidence

Rendered official pages, captured with `tools/parity/rea-page.sh`:

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/avfaudio (AVFAudio, framework landing page) | `ev_a3cbcb96e8653df1c25f58988cedcfbe12d1502d4df0cd7742a1fbd7f5c4579b` |
| https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer | `ev_86a904ae48d78002c9324dee374d4c2ca357ae0d82124aa577b1ea83cc8250ab` |
| https://developer.apple.com/documentation/avrouting/avroutingplaybackarbiter | `ev_fb8f46a630958f745e74d74ee74666271e6280aefbdb5912db7e0646b9fa640c` |
| https://developer.apple.com/documentation/coreaudio/audiohardwaresystem | `ev_527345d58b3e6297b8333562f55f83589d7d559a9e12cc2a706304fd2de010c6` |
| https://developer.apple.com/documentation/coreaudio (Core Audio, framework landing page) | `ev_f38fa930f96dad38a44aac5e365ee3d9ee51dcb19eb096a78f18d76d7845aaa7` |

The `ev_5273...` id was captured from the Audio Hardware System URL in the same batch; its rendered text was not
inspected separately.

## Matrix rows closed

Status values are the proposed ones for the lead to apply centrally. The matrix itself was not edited.

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| avfaudio | Audio session (AVAudioSession) | partial | `springboard/ui/src/audio_session.rs` |
| avfaudio | Audio interruptions | partial | `springboard/ui/src/audio_session.rs` (`begin_interruption`, `end_interruption`, `take_events`) |
| avfaudio | Speech synthesis (AVSpeechSynthesizer) | partial | `springboard/ui/src/speech.rs` |
| avfoundation-capture-audio | Speech synthesis | partial | `springboard/ui/src/speech.rs` |
| application-services | Speech Synthesis Manager | partial | `springboard/ui/src/speech.rs` (`voices_for_language`, `Synthesizer::is_speaking`, rate and volume on `Utterance`) |
| avrouting | Playback arbitration between participants | partial | `springboard/ui/src/playback_arbiter.rs` |
| core-audio | Boxes and device containers | partial | `springboard/ui/src/audio_hardware.rs` (`DeviceBox`) |
| core-audio | Aggregate devices | partial | `springboard/ui/src/audio_hardware.rs` (`AggregateDevice`) |
| core-audio | Process objects (client process list) | partial | `springboard/ui/src/audio_hardware.rs` (`AudioProcess`, `ProcessList`) |

Not closed in this round, on purpose:

- Speech synthesis buffer output (`write`), boundary pause and stop, and SSML and IPA input: no audio engine exists
  to apply them to.
- Core Audio process taps and the Audio Server driver plug-in: these need an output-side hook in the audio service.
- Speech recognition rows: no transcription model was added in this theme.

Partial rather than parity: each module covers the state and rules, but none is connected to the audio service yet,
so no app can use them through the SDK.

## Tests

Each changed or new module has a `#[cfg(test)] mod tests` with at least five behaviour tests:

- `springboard/ui/src/speech.rs`: 3 new tests (`pre_delay_clamps_bad_values_like_post_delay`,
  `speaking_and_paused_queries_follow_the_state`, `voices_for_language_lists_exact_matches_before_primary_matches`)
  added to the existing set.
- `springboard/ui/src/audio_session.rs`: 6 tests.
- `springboard/ui/src/playback_arbiter.rs`: 5 tests.
- `springboard/ui/src/audio_hardware.rs`: 6 tests.

The tests were not built or run in this round (the task forbids builds in this worktree). `python3 tools/check-names.py`
exits 0 with no hits.
