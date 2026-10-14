# AVFAudio: parity pass (team n)

Official reference: https://developer.apple.com/documentation/avfaudio

This pass closes the app-facing gaps in the AVFAudio matrix rows with two pure, std-only models. Neither touches a
device, audiod or unsafe code. Both are built on branch `parity/team-n-avfaudio`; the rows stay `partial` until they
are verified on Midget.

## What was built

1. `springboard/sdk/src/audio_application.rs` (bifrost_app, module `audio_application`). Mirrors the application-level
   state of the reference:
   - `RecordPermission` (`Undetermined`, `Denied`, `Granted`) and `request_record_permission`, which calls the
     supplied prompt only while the permission is undetermined and returns the stored answer afterwards.
   - `set_permission_from_settings`, standing in for the user changing the choice in system settings.
   - Input mute: `set_input_muted` reports real changes only, and `take_mute_events` returns the queued changes.
   - `may_capture`, true only when permission is granted and input is not muted.
2. `springboard/sdk/src/audio_player_control.rs` (bifrost_app, module `audio_player_control`). The controls that the
   player adds on top of play, pause, stop and volume:
   - `clamp_pan` (-1 to 1), `clamp_rate` (0.5 to 2.0) and `clamp_volume` (0 to 1). NaN becomes silence for volume,
     centre for pan and normal speed for rate.
   - `VolumeFade`, a linear ramp from the volume in effect to a target over a duration in milliseconds. A zero-length
     fade is immediate, and the arithmetic saturates at the end of time.
   - `PlayerControl`, which holds volume, pan, rate and an optional fade. `set_volume` cancels a fade, and a new fade
     starts from the volume in effect at its start time.

## Not done here

- Neither model is wired to audiod or to the playback path. The player and the recorder keep their current behaviour.
- The session model in `audio_session.rs` (categories, interruptions, route changes) already exists and is not changed
  in this pass. Wiring it to the shell and audiod remains open.
- MIDI playback, the AVAudioEngine node graph and speech synthesis rows are unchanged.

## Tests

`permission_is_asked_once_and_the_answer_sticks`, `a_refused_prompt_denies_and_does_not_ask_again`,
`denied_permission_changes_only_through_settings`, `mute_reports_real_changes_only`,
`capture_needs_permission_and_an_unmuted_input` in `audio_application.rs`;
`pan_and_rate_are_clamped_and_nan_safe`, `player_defaults_and_setters_clamp`,
`fade_ramps_linearly_and_settles_on_the_target`, `a_zero_length_fade_is_immediate`,
`a_new_fade_starts_from_the_volume_in_effect`, `set_volume_cancels_a_fade`,
`fade_arithmetic_does_not_overflow_at_the_end_of_time` in `audio_player_control.rs`.

These have not been built or run in this pass. The lead builds and runs them on the host, then verifies the rows on
Midget.
