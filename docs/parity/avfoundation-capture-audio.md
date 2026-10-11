# AVFoundation (Capture, Audio): parity pass

Official reference: https://developer.apple.com/documentation/avfoundation (speech synthesis collection:
https://developer.apple.com/documentation/avfoundation/speech-synthesis, audio playback, recording and processing:
https://developer.apple.com/documentation/avfoundation/audio-playback-recording-and-processing).

## What was built

Two pure, std-only models. Neither touches a device, a service or unsafe code.

1. `springboard/ui/src/speech.rs` (bifrost-ui, module `speech`). Mirrors the speech synthesis shape of the reference:
   - `Utterance`: text, rate, pitch multiplier, volume, post-utterance delay and a language tag. Setters clamp into
     the ranges, non-numeric values fall back to the defaults, and a negative delay becomes zero.
   - `Voice` and `select_voice`: exact BCP 47 match ignoring case and `_`/`-`, then the first voice with the same
     primary subtag (so `en-US` can use an `en-GB` voice).
   - `Synthesizer`: a queue spoken in order with states `Idle`, `Speaking`, `Paused`; `speak`, `pause`, `resume`,
     `stop` and `finish_current`, where `pause`, `resume` and `stop` report whether they changed anything.
2. `springboard/core/src/meter.rs` (core, module `meter`). Per-channel peak and average power of interleaved 16-bit
   PCM, in dBFS with a `-160` floor, for level meters. `measure` ignores a trailing partial frame and returns no
   channels when given zero.

Tests: `settings_are_clamped_into_their_ranges`, `non_numeric_settings_fall_back_to_defaults`,
`voice_matches_exactly_then_by_primary_subtag`, `queue_speaks_in_order_and_goes_idle_at_the_end`,
`pause_and_resume_only_change_state_from_the_right_one`, `stop_clears_the_queue_and_reports_whether_anything_was_running`
in `speech.rs`; `silence_reads_as_the_floor`, `full_scale_peak_is_near_zero_dbfs`,
`half_scale_constant_is_about_minus_six_db`, `channels_are_measured_separately`,
`average_is_below_peak_for_a_varying_signal`, `trailing_partial_frame_and_zero_channels_are_handled`,
`to_db_handles_bad_input_without_panicking` in `meter.rs`. These have not been built or run in this pass; a later
stage runs them.

## Official API names mirrored

- Speech synthesis: utterance (`rate`, `pitchMultiplier`, `volume`, `postUtteranceDelay`), voice by language
  (`AVSpeechSynthesisVoice(language:)` as used in the reference sample), synthesizer (`speak`, pause, continue, stop).
- Recording level metering: the per-channel peak and average power that the recorder exposes. The reference recorder
  page was not retrievable at its symbol path in this pass, so the metering names rest on the matrix row, not a
  fetched symbol page.

## Evidence

- `ev_5e0aff067e35acee7f9a8fdcbc151e7a5b97f642400a5701b932d4d8776c83c9`: REA rendered capture of the Speech synthesis
  page. Confirms the framework's two-task model (create an utterance, pass it to a synthesizer), the utterance
  properties used in the sample (`rate`, `pitchMultiplier`, `postUtteranceDelay`, `volume`), and the voice lookup by
  language.
- `ev_66ac959170746ca4ea8b642ceb6488c15f27192d7d187706f506a7b6efe1a581`: REA capture of the `avaudiorecorder` symbol
  path. The page returned "can't be found", so it supports nothing about the recorder. Recorded for transparency.

## Limits of this pass

- The `docc.py` JSON lookup returned 404 for the symbol paths (`avspeechutterance`, `avaudiorecorder`). The clamp
  ranges in `speech.rs` (rate 0 to 1 with 0.5 default, pitch 0.5 to 2 with 1.0 default, volume 0 to 1) follow the
  documented limits as understood, but they are not confirmed against the symbol pages. Check them on
  developer.apple.com before the row is marked `parity`.
- Nothing produces audio. `speech.rs` has no engine, no SDK export and no caller.
- The meter is not called by the recorder in `springboard/src/media/capture.rs`, and no meter view exists.

## Remaining gaps

- Speech engine that produces audio, and an SDK API (`bifrost_app`) for apps to speak.
- Wiring the level meter into the recorder and exposing its levels.
- Recording settings (format, sample rate, channels as a settings dictionary) and compressed formats.
- App-facing capture session and devices, camera and microphone permission prompts.
- Audio engine graph, video file recording, sample buffers, depth and metadata outputs.
