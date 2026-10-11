# Theme 17: clip buffering and recording editor

Cross-framework theme: clip buffering, recording sessions, camera choice and trimming of a finished recording. Covers
the ScreenCaptureKit clip-buffer and recording-editor outputs, the ReplayKit recorder, and the trim and camera rows
that the other frameworks in this theme list.

## What was built

Two pure Rust modules in `springboard/ui/src` (bifrost-ui, std only, no capture or file I/O). Neither reads a clock;
callers pass times in seconds as `f64`.

- `springboard/ui/src/clip_buffer.rs` (`ClipBuffer<T>`, `Frame<T>`, `ClipError`): a rolling buffer of already-encoded
  frames. It is started and stopped, refuses frames that are out of order, and keeps about `span` seconds. Eviction
  never leaves a non-keyframe at the front. `export(duration)` returns the frames of the last `duration` seconds,
  starting at a keyframe, so a clip can be a little longer than asked for. Durations past the span are cut to it.
- `springboard/ui/src/recording.rs` (`Recorder`, `Event`, `Finished`, `RecordError`, `CameraPosition`, `Trim`,
  `MIN_TRIM`):
  - `Recorder` holds availability, the running session, the microphone flag, the camera choice (`Front`, `Back`, or
    off), and an event queue drained with `take_events()`. Losing availability or an owner `fail(...)` ends a session
    with a `Failed` event.
  - `Trim` is the kept range of a finished recording. Its start and end handles stay at least `MIN_TRIM` (0.1 s)
    apart and inside the recording. Non-finite values are ignored.

Nothing here is wired into the shell, the SDK or the compositor yet. No frame source feeds `ClipBuffer`, and no app
can reach `Recorder` through the SDK. Those are the next steps.

## Official API names mirrored

Taken from the official DocC JSON (`tools/parity/docc.py`) and the rendered pages (REA evidence below).

- ReplayKit `RPScreenRecorder`: `isAvailable`, `isRecording`, `isMicrophoneEnabled`, `isCameraEnabled`, `cameraPosition`
  with `RPCameraPosition`, `delegate` / `RPScreenRecorderDelegate` (as the event queue), `startRecording(withMicrophoneEnabled:handler:)`,
  `stopRecording(handler:)`, `discardRecording(handler:)`, `startClipBuffering(completionHandler:)`,
  `stopClipBuffering(completionHandler:)`, `exportClip(to:duration:completionHandler:)`.
- ScreenCaptureKit `SCClipBufferingOutput`: `exportClip(to:duration:completionHandler:)`. Its clip is cut from the
  buffered frames, which is what `ClipBuffer::export` does, without the stream or the file write.
- ScreenCaptureKit `SCRecordingEditor`: the editing decision (keep a range of the recording). Its presentation API
  (`present(from:mode:completionHandler:)`) is not built.

The REA page for ReplayKit's `RPScreenRecorder` marks the class **Deprecated**, with the note "Use ScreenCaptureKit
instead". The DocC JSON does not show that flag, so the matrix note that called it unconfirmed is now resolved by the
rendered page.

## REA evidence

- `ev_b201efcf4e8a3f1252fa53d9d15076c649b38d7cb712784e89213203db7b63f9`: rendered page
  `https://developer.apple.com/documentation/replaykit/rpscreenrecorder` (Deprecated, "Use ScreenCaptureKit instead").
- `ev_1edc5c929e30e77cc954cae897ca4e91ecb6ad57cf7de3992a8045bb5733511e`: rendered page
  `https://developer.apple.com/documentation/screencapturekit/scclipbufferingoutput`.

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| screencapturekit | Clip buffering and recording editor (SCClipBufferingOutput, SCRecordingEditor, SCVideoEffectOutput) | partial | springboard/ui/src/clip_buffer.rs, springboard/ui/src/recording.rs (Trim) |
| replaykit | App screen recording (RPScreenRecorder start/stop) | partial | springboard/ui/src/recording.rs (Recorder) |
| replaykit | Clip buffering and export | partial | springboard/ui/src/clip_buffer.rs (ClipBuffer::export) |
| replaykit | Capture delegate and availability | partial | springboard/ui/src/recording.rs (Recorder::is_available, set_available, take_events) |
| replaykit | Camera preview during recording | partial | springboard/ui/src/recording.rs (CameraPosition, Recorder::set_camera); no preview view |
| replaykit | Preview and edit UI for a recording | partial | springboard/ui/src/recording.rs (Trim); no preview or trim sheet |

Still open in these rows: no frame source feeds the buffer, no SDK type exposes recording to apps, the camera
preview view is absent, and the preview and trim UI is absent. Screen Time, the Screen Saver, and the other
frameworks in this theme (AVKit trimming, Video Toolbox, Media Intelligence, Automator) are not touched.

## Tests

`cargo` was not run (the wave rules forbid builds). The tests live in each module: `clip_buffer.rs` has 7 tests
(span validation, refusal rules, eviction and keyframe start, export start and clamp, export errors, stop/clear) and
`recording.rs` has 8 tests (availability, session start/stop/discard, camera busy and availability loss, owner
failure, trim validation and handle clamping).
