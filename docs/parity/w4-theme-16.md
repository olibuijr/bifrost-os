# Wave 4, theme 16: session properties, encoder list, scheduled regions, hardware clocks

Theme: "Session properties" (size 11; frameworks Video Toolbox, Audio Toolbox, Core Audio). This wave builds three pure
models in `springboard/ui/src` (`bifrost-ui`). None of them is wired into the audio service, the shell or an SDK
module yet, and none has run on the VM or Midget. The code has not been built or tested in this wave (no builds were
run); the tests were checked by hand.

## What was built

| Module | What it models | Official API it mirrors |
|---|---|---|
| `springboard/ui/src/video_session.rs` | `PropertySet`: named session properties with a kind (flag, integer range, or list of names), a default, and a write permission. `copy_property` returns the value or the default. `set_property` refuses unknown, read-only, wrong-kind and out-of-range values. `publish` lets the host set read-only properties. `supported` returns the property dictionary in name order. `Catalogue` registers encoders by identifier and codec and opens a session with an encoder's properties. | VideoToolbox `VTSession`, the set/copy property calls, `VTCopySupportedPropertyDictionaryForEncoder`, `VTCopyVideoEncoderList` |
| `springboard/ui/src/scheduled_region.rs` | `FileRegion` (a span of an open file, with `frames_to_play`, `loop_count` where 0 does not loop, and a start time on the sample timeline). `Slice` (one channel of samples in memory). `Schedule` renders an output block into `Piece`s that say which source frames play at which offset, splits blocks at loop edges, and reports each finished item once. `cancel` removes a queued item. | `ScheduledAudioFileRegion` (`mStartFrame`, `mFramesToPlay`, `mLoopCount`, `mTimeStamp`), `ScheduledAudioSlice` |
| `springboard/ui/src/hardware_clock.rs` | `HardwareClock`: a clock with a uid, a clock domain, living and running state, a nominal rate and the rate ranges it accepts, input and output latency in frames. It maps sample time to host time and back, from the first and latest observed pairs, and measures the rate the device actually runs at. `sync_peers` lists the living clocks in the same clock domain. | `AudioHardwareClock` (`clockDomain`, `isRunning`, `nominalSampleRate`, `availableNominalSampleRates`, `inputLatency`, `outputLatency`, `isAlive`, `uid`, `setNominalSampleRate`) |

Each module is registered with one `pub mod` line in `springboard/ui/src/lib.rs`.

## Apple references

- `AudioHardwareClock`, DocC https://developer.apple.com/documentation/coreaudio/audiohardwareclock.
  The DocC page states that clocks in the same `clockDomain` "are able to be synchronized in hardware" and that
  `isRunning` false means the device "is not providing timestamps". REA evidence: `ev_1f00c58ee958299b4f2b7c944323175f674eaf02a2e6e5c91908bed0fac75e10`.
- `ScheduledAudioFileRegion`, DocC https://developer.apple.com/documentation/audiotoolbox/scheduledaudiofileregion.
  The DocC page gives `mLoopCount` ("0 = do not loop"), `mFramesToPlay` ("The number of frames to play") and
  `mStartFrame` ("The frame offset into the file"). REA evidence: `ev_51b47cf3f36a0c7398802c7b892e696cde6ff710ae3ceddd2cb5a2e5fd55993d`.
  The DocC page for `ScheduledAudioSlice` gives only its member names, so the slice model is a deliberate reduction:
  one channel and no completion callback.
- `VTSession`, DocC https://developer.apple.com/documentation/videotoolbox/vtsession ("A reference to a VideoToolbox
  compression session, decompression session or pixel transfer session."). REA evidence: `ev_02e8040e7af3bc6c928bc7dc971ad9dc2e597de12826d2d968f295fa35e8166b`.
  The DocC index lists `VTCopyVideoEncoderList(_:_:)` ("Builds a list of available video encoders.") and
  `VTCopySupportedPropertyDictionaryForEncoder(...)` ("Builds a list of supported properties and encoder ID for an
  encoder."). The DocC path for `VTSessionSetProperty` and `VTSessionCopyProperty` returned 404 from `docc.py`, so the
  set and copy behaviour of `PropertySet` follows the matrix's reference names and the index entries above, not the
  rendered page of each call. This is the weakest part of the evidence.
- REA ran once per page on the three pages above. The rendered text matches the DocC abstracts.

## Gaps left open

- No encoder exists in BifrOSt. The catalogue holds encoders that a backend registers. Nothing encodes, and the
  ffmpeg path keeps its options internal. The matrix's "depends on the encode gap" note still applies to the encoder
  rows.
- The hardware clock has no suggested reference device (`kAudioDevicePropertySuggestedReferenceDevice`). The DocC page
  for that property was not fetched in this wave, so its selection rule is not implemented.
- Scheduled regions are mono and have no completion callback. The audio service still plays whole tracks.
- Nothing in the shell or SDK uses these models yet. No app can reach them.

## Matrix rows closed (status `partial`)

Rows are for the lead to merge into the central matrix. Their `feature` text is copied from `docs/parity-matrix/data/<slug>.json`.

| Framework slug | Feature (as written) | New status | BifrOSt path |
|---|---|---|---|
| video-toolbox | Session properties | partial | springboard/ui/src/video_session.rs (`PropertySet`) |
| video-toolbox | Encoder list and property query | partial | springboard/ui/src/video_session.rs (`Catalogue`, `EncoderInfo`) |
| audio-toolbox | Scheduled file regions and slices (ScheduledAudioFileRegion) | partial | springboard/ui/src/scheduled_region.rs (`FileRegion`, `Slice`, `Schedule`) |
| core-audio | Hardware clocks and sample-accurate sync | partial | springboard/ui/src/hardware_clock.rs (`HardwareClock`, `sync_peers`) |

Not closed in this theme: Audio Toolbox "Voice I/O" (an echo canceller is still missing) and the encode-dependent
video rows.
