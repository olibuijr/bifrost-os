# Voice Memos and audio recording (slug: voice-memos-audio)

Round 75. Clean-room notes. The macOS 27 Voice Memos bundle is not yet extracted under
`~/Projects/macos-ref/extract/ref27/` (only the index lists `System/Applications/VoiceMemos.app`), so this round
worked from the existing code, the docs and the matrix rows. REA evidence for the Voice Memos bundle is still to
be taken once the lead extracts it (Info.plist, the Recording menu and the settings extensions).

## Matrix rows touched

| Row (data file) | API | BifrOSt path | Done this round | Still needs |
|---|---|---|---|---|
| Audio recording (avfoundation-capture-audio) | AVAudioRecorder-style capture | `springboard/src/media/capture.rs`, `springboard/audiod/src/main.rs`, `springboard/apps/voicememos-desktop` | Enhance Recording: a noise-gated copy of a take (`apps/voicememos-desktop/src/enhance.rs`, `store::save_enhanced`, Recording menu) | Recording quality and compressed formats are not in the service; still partial |
| Basic recording (avfaudio) | AVAudioRecorder | same | as above | no pause, no metering API for apps |
| Audio recording, input queue (audio-toolbox) | Audio Queue input | same | as above | no raw sample callback for apps |

## Not closed this round

- Playback speed: the audio service (`springboard/src/media/audioplayer.rs`, `Engine::write_samples`) resamples to
  `OUT_RATE` with `Resampler::retune`. A pitch-preserving rate needs a time-stretch stage, plus a new `Cmd` in
  `springboard/proto/src/svc.rs` and position maths that accounts for the rate. Shared with Music and Podcasts, so it
  is not done in this slice.
- Sound input level meter in Settings > Sound: `springboard/src/shell/sound_page.rs` has the "Input Level" row and
  `SoundView.level`, but nothing fills `SoundView` (no producer in `settings.rs` or `audiosvc.rs`), and
  `set_input_active` has no caller. A meter-only mode needs a new service command (the current `RecordStart` writes a
  file on stop), a shell client in `audiosvc.rs`, and a few lines in `settings.rs`.
- Recording library, waveform, trim, rename, split, skip silence and notes already exist in
  `apps/voicememos-desktop`; no change this round.

## Verification status

Not built or run (round rule: code and commit only). Unit tests added: `enhance::tests` (gate reduces hiss, keeps
speech, holds word tails, keeps stereo balance, leaves silence and partial frames alone), `store::tests`
(`enhanced_copy_keeps_the_length_and_the_original`), `voicememos::tests` (`enhance_saves_a_gated_copy_and_selects_it`).
Needs a host test run and a Midget check of the Recording menu item.
