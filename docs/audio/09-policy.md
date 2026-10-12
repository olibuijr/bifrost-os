# Audio 09: policy, Settings > Sound, input indicator

## Implemented

`springboard/audio/policy` is now a library plus the binary. The library holds pure state machines with host tests:

- `defaults.rs`: default device per role (output, input, sound effects), the chosen device survives its absence,
  first able device otherwise, jack rule (headphones in: switch and remember the previous output; out: restore and
  `pause_media`), `key=value` persistence.
- `ducking.rs`: notification and speech reasons, gain ramps to 0.3 (attack 0.2, release 0.05 per tick).
- `permissions.rs`: microphone, audio-capture, audio-exclusive per app. Unknown app: `Prompt` once, answer stored
  (tab-separated file). Input indicator is on while any input stream or tap runs.
- `volume.rs`: per-app volume, clamped 0..1.
- `main.rs`: loads prefs and logs (`is.olibuijr.audio`, category `policy`). Logging is stderr, prefixed.

Shell: `src/shell/sound_page.rs` builds the OUTPUT and INPUT groups (with the input level row) from a `SoundView`;
`pages.rs` appends them to the Sounds page via `Model.sound`; `Pick::SoundOutput/SoundInput` store
`audio_default_output` / `audio_default_input` in prefs; `desktop_bar.rs` draws an orange dot on the Control Center
glyph while `sound_page::input_active()` is set.

`ui/src/audio_session.rs`, `default_device.rs` and `media/route.rs` were already complete and are unchanged; the
policy library re-implements their logic against UIDs so the process does not depend on the UI crate.

## Left (todo_left)

- The control-socket loop in `main.rs` (Hello with `policy: true`, Listen on `dev#`, `jack`, defaults; Set the
  defaults) needs the HAL client (task 06) and the server (05).
- The audio service must fill `SoundView` and call `set_input_active`; the shell prompt for the microphone.
- Per-app volume and ducking gains need the mixer hook (task 05/07).
- Live level meter row is a percentage label, not a bar.

## Midget verification (lead)

1. `bifrost pc smoke Settings --sizes 1200x780 --scale 1.5625` and with `--light`; open Sounds.
2. After wiring: simulate the jack with the ctl test hook, check `bifrost pc audio get system dOut` changes to the
   headphone device and the media player pauses on unplug.
3. Start a recorder (Voice Memos): the prompt shows once; the orange dot shows on the menu bar while recording.
4. `cargo test -p bifrost-audio-policy` and `cargo test -p springboard sound_page`.
