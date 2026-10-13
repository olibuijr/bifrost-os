# Parity slice: quicktime-player (Video Player)

Slice: the player-and-recorder app row (`path:/System/Applications/QuickTime Player.app` in `docs/parity-matrix/os/apps/p-r.json`)
and the AVKit, AVFoundation playback and editing, ReplayKit and ScreenCaptureKit rows it depends on.

App: `springboard/apps/videoplayer-desktop` (crate `bifrost-videoplayer`, id `is.olibuijr.videoplayer.desktop`, shown as
"Video Player"). The name avoids the reference product name in code and in the app's user-facing strings.

## Reference evidence

- Shipped bundle index: `~/Projects/macos-ref/extract/ref27-index.tsv` (325 entries under the player app's path, including
  `Contents/Info.plist` and `Resources/Base.lproj/*.nib`, for example `FinishingRecordingPanel.nib`).
- The bundle itself is not extracted yet under `~/Projects/macos-ref/extract/ref27/System/Applications/` (checked during
  this round). Menus, document types and the recording-panel layout are not yet measured with `inspect_plist` or
  `decode_interface_builder`. Re-check and record REA evidence ids here once the extract lands.
- Behaviour used for the design came from the existing BifrOSt code, the matrix notes and the public names of the
  reference rows, not from the reference binaries.

## Rows

### AVKit: standard player (`Standard player`)
- API: player view controller and the playback HUD.
- BifrOSt path: `springboard/src/deildu/player_view.rs` (`PlayerView`, HUD, scrubber, double-tap skips), hosted by
  `springboard/apps/videoplayer-desktop/src/movie.rs` over `springboard::shell::video::MediaPlayer`.
- Done: the app plays a local movie through the existing fullscreen view; keys and touch go through `PlayerView::key` and
  `PlayerView::touch`.
- Needs Midget: the HUD on a real touch screen and the hardware keys; start and stop at a real movie file.

### AVKit: captions and subtitle tracks
- API: subtitle options on the player, legible text with the cue on screen.
- BifrOSt path: `springboard/src/media/subtitles.rs` (`Track::parse`, `Track::text_at`, `sidecar_candidates`); drawn by
  `movie.rs` (`draw_caption`) and switched from the Movie menu (`Subtitles`, checked state).
- Done: SRT and WebVTT parsing (tags, CRLF, BOM, settings, broken cues skipped); sidecar `.srt` or `.vtt` next to the
  movie; cue lookup with start inclusive and end exclusive. Unit tests in `subtitles.rs` and `movie.rs`.
- Gaps: no subtitle track choice inside a container (embedded text tracks are not demuxed); captions are not picked for
  movies opened through the document panel, because the panel gives the app a file handle and no path (the lookup is by
  path).
- Needs Midget: legibility of the caption plate over bright and dark frames.

### AVKit: Picture in Picture
- API: a floating picture that stays above other windows and follows the user.
- BifrOSt path: `springboard/src/media/pip.rs` (`size`, `place`, `nearest_corner`, `drag`, `centre`).
- Done: the geometry model (size from the movie aspect, minimum width, corner placement with margin, snap to the nearest
  corner on release, drag clamped to the viewport). Unit tests in `pip.rs`.
- Gaps: no always-on-top or floating window in the shell for an app (the shell's `pip_auto` setting is stored and not
  read, see the AVKit row note). The app does not enter Picture in Picture yet. This row stays Gap until a window level
  API exists.

### AVFoundation (Playback, Editing, Common): trim and export
- API: time ranges, the editing model and export.
- BifrOSt path: the shared `Trim` model in `springboard/ui/src/recording.rs` (`bifrost_app::controls::recording::Trim`),
  used by `movie.rs` (`set_trim_start`, `set_trim_end`, `clear_trim`, `wrap_trim`).
- Done: the trim range from the playhead (the shared model, with its minimum length and clamping); playback loops inside a
  partial trim; the Movie menu sets and clears it.
- Gaps: no export. There is no movie writer (muxer) in the media stack, so the trimmed range cannot be saved as a file.
  The export menu item is not listed until a writer exists.
- Needs Midget: the loop at the trim end, and the first-frame timing after a seek.

### ReplayKit: recording (audio take)
- API: start and stop recording with a sample stream.
- BifrOSt path: `springboard/ui/src/recording.rs` (`Recorder`, `Finished`, `Event`, `RecordError`) for the session state;
  `bifrost_app::audio::Client::record_start` and `record_stop` (the audio service, the same call Voice Memos uses) write
  the WAV file into the app's data folder.
- Done: New Audio Recording and Stop Audio Recording from the Recording menu; the recorder session state (started,
  stopped with a duration, failed when the service refuses); a refused start shows the reason in the window's notice.
- Gaps: no microphone permission prompt inside this app (Voice Memos checks the privacy service before recording; this
  app does not yet). The shell's recording panel is not used. The take is not added to a library; it is a file in the
  app's data folder.
- Needs Midget: the audio service answers `record_start` for an app that was not armed by the shell (Voice Memos creates
  its client the same way; confirm the take file is written).

### ReplayKit: screen recording
- API: screen video, app audio and microphone samples.
- BifrOSt path: `Recorder::new(false)` in `springboard/apps/videoplayer-desktop/src/main.rs` (`screen`); the menu item
  New Screen Recording is listed and disabled because `is_available()` is false.
- Gaps: no screen capture stream exists for apps (see ScreenCaptureKit below), so screen recording cannot start. The
  item becomes enabled when a capture service sets the recorder available.

### ScreenCaptureKit
- Unchanged by this round. The app consumes the recorder model only; the capture stream, the content filters and the
  shareable content are still missing (see `docs/parity-matrix/data/screencapturekit.json`).

## QuickTime Player app row (`os/apps/p-r.json`)

Partial. The Video Player app covers playing a local movie, captions from a sidecar file, trim loops, and an audio take.
It does not cover screen recording, export of a trimmed movie, Picture in Picture, a movie library, streaming or the
recording panel. The row stays partial.

## Matrix entries

No entry is closed by this round. Entries advanced but not closed: the QuickTime Player app row, the AVKit standard player
and captions rows, the AVFoundation playback and editing row (trim), and the ReplayKit row (audio take). The lead should
decide closures after Midget checks.

## Tests

Unit tests are in `springboard/src/media/subtitles.rs`, `springboard/src/media/pip.rs` and
`springboard/apps/videoplayer-desktop/src/movie.rs`. They were written by reading the code and have not been compiled in
this round (builds are run by the lead).
