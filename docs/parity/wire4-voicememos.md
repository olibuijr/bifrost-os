# Wave 4 wiring: Voice Memos (1.5.0), audio

App: `springboard/apps/voicememos-desktop/` (`bifrost-voicememos`, now 1.5.0 in `Cargo.toml` and `manifest.toml`).
Branch `wire4/voicememos`. No builds or tests were run in this round; the lead builds once. The host tests added here
were worked out by hand and are listed below, not run.

Focus of this round: the audio rows (basic recording, basic playback, mixing and volume, microphone capture, property
change notifications). Most of the audio path was already wired in earlier rounds: the microphone permission gate, the
input device name and the change notice (`audio-devices` `devices_changed`), the format line and trim. This round adds
the input meter and the playback volume slider, and records the two things the audio service cannot do yet.

## What is now wired

| Matrix slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | Model or service | BifrOSt path |
|---|---|---|---|
| `avfaudio` | Basic recording (AVAudioRecorder) | `bifrost_core::meter::to_db` (dB scale, floor -160 dB), the service's `Evt::Record` level | `src/voicememos.rs` `level_db`, `meter_fraction`, `meter_title`; `draw` (recording branch, the meter under the timer); `sync_audio` (`input_level`) |
| `avfaudio` | Mixing and volume control | `bifrost_app::controls::values::Slider` over the service's output volume (`audio::Client::set_volume`, `snapshot().volume`) | `src/voicememos.rs` `volume_rect`, `volume_at`, `set_volume`, `draw_volume`, `Target::Volume` in `hit` and `touch`; `sync_audio` (follows the service level) |
| `avfaudio` | Basic playback (AVAudioPlayer) | same volume slider (the `volume` property; `rate` and `pan` are not wired, see gaps) | as above |
| `core-audio` | Microphone capture | the service's `Evt::Record` level now drives a dB meter; the input device name is shown from `audio-devices` | `src/voicememos.rs` `draw` (meter), `set_input` (existing) |
| `core-audio` | Property change notifications | `audio-devices` `devices_changed` events (existing wiring, unchanged here): the caption names the input and a change during a recording is noted | `src/main.rs` `service_event`; `src/services.rs` `caption`; `src/voicememos.rs` `devices_changed`, `set_input` |

### Behaviour, for a user

- **Input meter**: while a recording runs, a bar sits under the timer. It fills on a decibel scale from silence (the
  left end, -60 dB) to full scale (0 dB), and the readout under it gives the level in whole dB ("-18 dB"), or "Silent".
  The bar is the app's one tint (blue). The live waveform above is unchanged.
- **Playback volume**: with a recording selected (and not recording), a slider sits under the -15 / play / +15 buttons.
  Press or drag it to set the output level, 0 to full. The slider shows the service's level, so a change made in Control
  Center moves it on the next tick. The level is the same one the shell's volume keys and Control Center change; it is
  not a per-recording volume.
- **Record button ring**: the ring around the red record button is now the content grey (`CONTENT_BG`), not the pure
  black `DETAIL_BG`, so it matches the window in Dark appearance.
- **Input device**: unchanged. The caption under the record button names the default input, and a device change during
  a recording says so (earlier wave).

## Models not wired, and why

- **`audio_settings::Settings` (recording quality: sample rate, channels, format)**: not wired. The service takes only a
  path (`Cmd::RecordStart { path }`); the recorder picks the first working PCM format itself. A quality setting in the app
  would not change the recording, so it is not shown. Wiring it needs a `format` field on `RecordStart` in
  `springboard/proto/src/svc.rs` and the recorder in `springboard/src/media/capture.rs`, which are outside this app.
- **`playback_rate::RateCommand` (playback speed)**: not wired. The audio service has no rate command or field (`Cmd` has
  `SetVolume`, `SetMuted`, `SeekMs` and no rate). The model can pick the nearest supported speed, but nothing plays faster.
  Needs a `SetRate` command in the service.
- **`volume_control::VolumeControl`**: not used by the app. The slider writes the service's own level directly, so the
  model's level and mute would be a second copy of the state. Mute is not in the slider (the service keeps it).
- **`audio_session`, `analyzer_input`**: not used. The session category is decided by the service, and the analyzer
  input is a time-coded buffer stream the service does not expose.
- **Route changes (`route_changed`)**: the app does not react to them. A headphone plug-in changes the default output but
  not the input the recording uses, so nothing in the recording path needs it.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Skip the VM.
2. `bifrost pc midget open-app is.olibuijr.voicememos.desktop`. Press **Record**. Speak or tap the microphone: the blue
   bar under the timer moves, and the readout changes from "Silent" to a dB value. Stop the recording.
3. Select the take. A volume slider appears under the -15 / play / +15 buttons, set to the Control Center level.
4. Drag the slider with `bifrost pc midget tap-window APP X Y` at the track's window-local pixels (or press and move with
   `swipe`). Open Control Center: its volume slider moved to the same level. Press **Play**: the output is louder or
   quieter as set.
5. Drag the slider to the far right and then the far left: it stops at full and at silence, it does not overshoot.
6. Dark appearance (`bifrost pc midget appearance dark`): the ring around the record button matches the content grey, not
   black.
7. `bifrost pc midget screen /tmp/voicememos-meter.png` for the screenshot record.

Host tests added (not run; expected values worked out by hand):

- `voicememos::tests::the_input_meter_reads_silence_as_the_floor_and_full_scale_at_the_top`: power 0 reads "Silent" and
  fraction 0; power 1 reads "0 dB" and fraction 1; power 0.25 (amplitude 0.5, -6.02 dB) reads "-6 dB" and fraction
  (60 - 6.02) / 60 = 0.8997; power 1e-7 (-70 dB) is below the floor, so fraction 0 and "Silent".
- `voicememos::tests::dragging_the_volume_slider_sets_the_level_and_clamps_at_the_ends`: a press a quarter of the way
  along the track sets 0.25; a drag past the right end clamps to 1.0; the drag ends on release.

The compliance snapshots (`tests/snapshots/voicememos-desktop-{dark,light}.png`) may show the record ring and, if a
recording is selected in the snapshot, the new volume slider. They need a bless (`BIFROST_BLESS=1`) after the lead's
build.

## REA and official-doc evidence

Official DocC (`python3 -I tools/parity/docc.py`) and rendered pages (`tools/parity/rea-page.sh`):

- AVAudioPlayer.volume (`avfaudio`, Mixing and volume control; Basic playback): rendered page
  `ev_f3d87da2d7efa1692eee44eef0c99b9420992f702b6a4131ab9aaf9764af23ae`. "The audio player's volume relative to other
  audio output", a value from 0 for silence to 1 for full volume.
- AVAudioRecorder metering (`avfaudio`, Basic recording): `ev_e177147fd2979690d36252d8567b45ef9912b40060c7003a22caddc3e6e8fbed`
  (from the wire-app-voicememos note; `averagePower` and `peakPower` in dBFS, the basis of the dB meter).
- AudioHardwareDevice and AudioHardwareSystem (`core-audio`, Property change notifications; Microphone capture):
  `ev_93cd1485507b2725b829b6e743dd267709255496d06e919f593b501523b1a1ec` and
  `ev_f573b51d47a70746544d7837da7ab84e0de746bf2f2ec18c21edee3396b75f73` (from the audio-devices wire note).
- The `avfaudio` feature row's own evidence: `ev_df0e56197670a298d99ca1c65c39630cfb6848bc2d4d337b0ef4e4f301f36f1d`.

## Remaining gaps

- **Recording quality (sample rate and format)**: needs `RecordStart` to carry a format and the recorder to honour it.
  Until then the format line in the detail subtitle shows what was actually recorded.
- **Playback speed**: needs a `SetRate` command in the audio service and a rate field in its snapshot. The model
  `playback_rate::RateCommand` is ready for the speed menu once it does.
- **Per-recording volume**: the slider sets the shared output level. A per-recording gain needs the service to
  play a track at a gain.
- **Input level in the matrix**: the meter reads the service's level, which is a single value per block. There is no
  per-channel peak from the service, so the meter shows the level, not a peak hold.
- **Input device selection**: the input is the default one. Choosing another input needs `set_default` from the app
  (the `audio-devices` service has it) and a control in the app; not added here.
- **Matrix rows**: `docs/parity-matrix/` is not edited in this round. The rows above should move to Partial only after
  the Midget check and the lead's update.

## Matrix rows

| Slug | Feature (as in the json) | BifrOSt path |
|---|---|---|
| `avfaudio` | Basic recording (AVAudioRecorder) | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`meter_fraction`, `meter_title`, `draw`) |
| `avfaudio` | Mixing and volume control | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`volume_at`, `set_volume`, `draw_volume`) |
| `avfaudio` | Basic playback (AVAudioPlayer) | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`volume_at`, `draw_volume`) |
| `core-audio` | Microphone capture | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`sync_audio`, `draw`), `src/services.rs` (`caption`) |
| `core-audio` | Property change notifications | `springboard/apps/voicememos-desktop/src/main.rs` (`service_event`), `src/voicememos.rs` (`devices_changed`, `set_input`) |
