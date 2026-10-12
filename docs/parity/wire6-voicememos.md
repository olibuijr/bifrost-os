# Wave 6 wiring: Voice Memos (1.6.0), recording and analysis

App: `springboard/apps/voicememos-desktop/` (`bifrost-voicememos`, now 1.6.0 in `Cargo.toml` and `manifest.toml`).
Branch `wire6/voicememos`, from main 0.5.43. No builds or tests were run in this round; the lead builds once. The host
tests added here were worked out by hand and are listed below, not run.

Focus of this round: the analysis side of recording (speech detection over a take, and splitting a take). The recording
format and enhanced-level rows are not wired: the audio service takes only a path for `RecordStart`, so the app cannot
change the format the recorder writes (see gaps).

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | Model | BifrOSt path |
|---|---|---|---|
| `speech` | Analyzer modules and voice activity detection | `bifrost_app::controls::voice_activity::SpeechDetector` (medium sensitivity, -45 dBFS, onset 2 frames, hangover 5 frames) | `src/store.rs` `speech_spans`, `next_speech_ms`; `src/voicememos.rs` `load_spans`, `toggle_skip_silence`, `skip_silence_from` (called from `sync_audio`) |
| `avkit` | Media trimming and export | `bifrost_app::controls::recording::Trim` (two parts, both at least `MIN_TRIM`) | `src/store.rs` `save_split`; `src/voicememos.rs` `split_selected`, `can_split`, menu `MENU_SPLIT` |

### Behaviour, for a user

- **Split at Playhead** (Recording menu, after Trim): while the selected take plays, the item is enabled. Choosing it
  stops playback and writes two recordings, `<title> part 1` and `<title> part 2`, at the point playback has reached.
  The original stays. The first part is selected. A split less than 0.1 s from either end is refused with
  "Cannot split: ..." in the error line. The item is greyed out when the selected take is not the one playing: the
  playhead only exists during playback, because stopping resets it.
- **Skip Silence** (Recording menu, a check mark after Export to Downloads): when on, a take plays only its speech.
  The speech spans are found once per play (the speech detector over 10 ms frames of the mixed-down WAV). While the
  playhead is in a quiet part, playback seeks to the start of the next speech. A take with no speech plays as before.
  Turning it on during playback takes effect at once. The setting is not saved between launches.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`. Skip the VM.
2. `bifrost pc midget open-app is.olibuijr.voicememos.desktop`. Record about ten seconds: speak for a few seconds, stay
   quiet for a few, speak again. Stop.
3. Recording menu > Skip Silence. Select the take and press Play. The playhead jumps over the quiet part; the speech
   plays. Turn Skip Silence off and play again: the quiet part plays.
4. Recording menu > Split at Playhead is greyed out while the take is stopped. Play it, wait a few seconds, choose
   Split at Playhead. Two new takes, `... part 1` and `... part 2`, appear, the first is selected, the original is kept,
   and the durations add up to the original.
5. Window input: `bifrost pc midget tap-window APP X Y` with window-local pixels (menus through the menu bar as usual).
6. `bifrost pc midget screen /tmp/voicememos-split.png` for the screenshot record.

Host tests added (not run; expected values worked out by hand):

- `store::tests::split_writes_two_parts_and_keeps_the_original`: an 8 kHz mono take of 1000 ms split at 0.25 s writes
  250 ms and 750 ms parts and keeps the 1000 ms original.
- `store::tests::a_split_too_close_to_an_end_writes_nothing`: a 0.05 s head is below the 0.1 s minimum, so `save_split`
  is an error and the list still has one take.
- `store::tests::speech_spans_find_the_loud_half_second`: 0.5 s silence, 0.5 s at amplitude 10000 (-10.3 dBFS), 0.5 s
  silence at 8 kHz gives one span from 500 ms to 1000 ms (within 0.5 ms).
- `store::tests::next_speech_jumps_over_silence_and_waits_inside_speech`: spans (500, 1000) and (2000, 2500): from 0 the
  next is 500; at 700 nothing (inside speech); at 1000 (the exclusive end) the next is 2000; at 2600 nothing is left;
  with no spans nothing.
- `voicememos::tests::split_at_the_playhead_writes_two_parts_and_selects_the_first`: the split is refused until the
  take plays, then the 250 ms playhead gives 250, 750 and 1000 ms takes and stops playback.
- `voicememos::tests::skip_silence_is_a_menu_check_and_jumps_over_the_quiet_part`: the menu item toggles on and off; with
  span (500, 1000), a position of 0 jumps to 500 and sets the guard to 500, 100 waits for the guard, 600 stays put, and
  with the setting off nothing moves.

The compliance snapshots (`tests/snapshots/voicememos-desktop-{dark,light}.png`) show the Recording menu only if the
snapshot opens it; the Recording menu now has two more items. They need a bless (`BIFROST_BLESS=1`) after the lead's
build if they fail.

## REA and official-doc evidence

- Speech detection (`speech`, Analyzer modules and voice activity detection; the row's own evidence):
  `ev_83d419f1903e2491c4d2f87ecace3e8a120a051be00d72c9640c0513b97e4722`. No new rendered page was captured in this
  round: the `docc.py` paths for `documentation/speech/speechdetector` returned 404, so the page was not re-read.
- Trimming by export (`avkit`, Media trimming and export; the split writes trimmed WAVs the same way):
  `ev_98898fb52f7856d5681302580007bace40fb726db8f8530c44922cadb31bd6aa` (AVAssetExportSession, from the wire-app note).

## Remaining gaps

- **Recording format choice (compressed or lossless)**: not wired. `audio_settings::Settings` can resolve a format, but
  the service's `Cmd::RecordStart` takes only a path, and the recorder (`springboard/src/media/capture.rs`) picks its own
  PCM format. A format choice in the app would not change the file. Needs a format field on `RecordStart` in
  `springboard/proto/src/svc.rs` and the recorder, which are outside this app.
- **Enhanced recording levels** (audio processing, echo cancellation, gain): not wired. `echo_cancel` and
  `audio_processing_tap` have no capture stage to plug into; the service has no processing command.
- **Split with no playback**: a split needs the take to be playing, so a split at an exact time (typed or dragged on
  the waveform) is not available. The waveform trim (earlier wave) is the way to cut a take at chosen points.
- **Skip Silence on the service side**: the app seeks the audio service; the service does not skip silence itself, so
  there is a short gap at each jump (one service seek). The spans are read from the whole file on play, so a very long
  take takes a moment to start.
- **Skip Silence with a non-16-bit file**: no spans, so the take plays in full.
- **Saved setting**: Skip Silence is not stored between launches.
- **Matrix rows**: `docs/parity-matrix/` is not edited in this round. The `speech` and `avkit` rows should move only after
  the Midget check and the lead's update.

## Matrix rows

| Slug | Feature (as in the json) | BifrOSt path |
|---|---|---|
| `speech` | Analyzer modules and voice activity detection | `springboard/apps/voicememos-desktop/src/store.rs` (`speech_spans`, `next_speech_ms`), `src/voicememos.rs` (`skip_silence_from`, `toggle_skip_silence`) |
| `avkit` | Media trimming and export | `springboard/apps/voicememos-desktop/src/store.rs` (`save_split`), `src/voicememos.rs` (`split_selected`) |
