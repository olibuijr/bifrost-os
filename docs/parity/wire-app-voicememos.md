# Wiring parity models into Voice Memos (1.3.0)

App: `springboard/apps/voicememos-desktop/` (`bifrost-voicememos`, version 1.3.0 in `Cargo.toml` and `manifest.toml`).
Branch `wire/app-voicememos`. Host tests were not run (no builds in this round); the lead builds once.

## What is wired

| Matrix slug (`docs/parity-matrix/data/`) | Feature (as in the row) | Model | BifrOSt path | Status |
|---|---|---|---|---|
| `replaykit` | Preview and edit UI for a recording | `bifrost_app::controls::recording::Trim` (`springboard/ui/src/recording.rs`) | `springboard/apps/voicememos-desktop/src/voicememos.rs` (`begin_trim`, `end_trim`, `trim_move`, `draw_trim`); `src/store.rs` (`trimmed_wav`, `save_trim`) | Wired in the app; still needs a Midget check |
| `avkit` | Media trimming and export | same `Trim` model, export as a new WAV | `src/store.rs` `trimmed_wav` / `save_trim` | Wired in the app (the row stays Gap until verified on Midget) |
| `avfaudio` | Basic recording (AVAudioRecorder) | recording format metadata, read from the WAV header | `src/store.rs` `pcm16_format`, `format_title`; `Memo.rate`, `Memo.channels` | Wired in the app (metadata only; no metering or settings) |

### Behaviour, for a user

- **Trim**: select a recording, then press **Trim** in the toolbar (between the empty space and Rename). The detail pane
  shows the waveform with two handles and "Keeps m:ss of m:ss". Press near a handle and drag; the nearer handle moves and
  the model keeps it inside the recording and at least 0.1 s from the other. **Done** writes the kept part as a new
  recording named `<title> trimmed` (numbered when taken) and selects it. The original is not changed. **Cancel** discards
  the trim. Playback stops when a trim starts. A recording shorter than 0.1 s says it is too short to trim. A trim over the
  whole recording saves nothing.
- **Format**: the detail subtitle reads `Wed 7 Oct  14:03  ·  8 kHz mono` (sample rate and channels from the WAV header).
  Files whose header is not 16-bit PCM show no format.

Trimming only reads and writes 16-bit PCM WAV, which is what the recorder writes. Other files show the trim as unavailable
through the same error path (`Cannot save the trimmed recording`).

## How to verify on Midget

1. Publish and update Midget (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`). Skip the VM.
2. `bifrost pc midget open-app is.olibuijr.voicememos.desktop`, record a short take with **Record**, stop it.
3. Select the take. The subtitle shows the format (for example `48 kHz mono`).
4. Press **Trim**. Drag the left handle right and the right handle left. Press **Done**. A new `... trimmed` recording
   appears at the top of the list, shorter by the trimmed amount, and the original is still listed.
5. Press **Trim** again and **Cancel**: nothing is added.
6. Window input: use `bifrost pc midget tap-window APP X Y` with window-local pixels for the toolbar and handles.
7. `bifrost pc midget screen /tmp/voicememos-trim.png` for the screenshot record.

Host tests added (not yet run): `store::tests::metadata_line_names_rate_and_channels`,
`store::tests::trimmed_copy_keeps_the_kept_samples_and_the_original`,
`voicememos::tests::trim_saves_the_kept_part_as_a_new_memo`.

## REA and official-doc evidence

Official DocC (`python3 -I tools/parity/docc.py`) and rendered pages (`tools/parity/rea-page.sh`):

- AVAudioRecorder (`avfaudio`): `ev_e177147fd2979690d36252d8567b45ef9912b40060c7003a22caddc3e6e8fbed`. Metering,
  `currentTime`, `pause`, `stop`, `deleteRecording`, channel assignments. Used for the metadata; the app records through
  the audio service, not this class.
- AVAssetExportSession (`avfoundation`, trimming by export): `ev_98898fb52f7856d5681302580007bace40fb726db8f8530c44922cadb31bd6aa`.
- RPPreviewViewController (`replaykit`, preview and edit of a recording): `ev_d92ca1738cada62eb3b8b206db8c121d0c4848c9cdbb5038f777727722949774`.
- AVPlayerViewController (`avkit`, docc only, no REA id): `showsPlaybackControls`, transport bar and scrub UI are shown
  by the system player; the app draws its own trim handles instead.

## Remaining gaps

- **Levels and waveform**: the app keeps its own square-root peak waveform (`store::peaks`) and live meter (`sqrt` of the
  service level). There is no model in `springboard/ui` for these, and `sound_envelope`, `audio_engine` and `analysis` are
  not used because they do not describe a recording's peaks. Not wired.
- **Metering API** (`averagePower`, `peakPower` in dBFS): the service reports one linear level; no dB model is wired.
- **Pause, scheduled start, format settings** (AVAudioRecorder): not in the service channel. Not wired.
- **Trim with the ruler**: handles are the only way to trim; no numeric entry, no undo, no per-channel trim.
- **Non-16-bit files**: not trimmed and not shown with a format.
- **Compliance snapshots**: `tests/snapshots/voicememos-desktop-{dark,light}.png` show the old toolbar and subtitle.
  They need a bless (`BIFROST_BLESS=1`) after the lead's build; the compliance test allows a small pixel drift, but the toolbar
  now has a third button.
- **Lock file**: `springboard/Cargo.lock` lists `bifrost-voicememos` at 1.2.3. It is outside this app folder and was not
  edited; the build refreshes it.
- Matrix rows: `docs/parity-matrix/` is not edited in this round. The rows above should move to Partial only after the
  Midget check and the lead's update.
