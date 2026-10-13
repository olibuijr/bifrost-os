# Displays and Sound made live (round 75, slice displays-sound-live)

Slice owner: displays-sound-live. Code only; the lead builds and checks on Midget. Builds and tests were not run.

Evidence: the wave 1 status doc (`docs/parity/settings-net-sound-display.md`) lists the open items for this slice.
The macOS 27 settings extension for Displays is in the reference index (`~/Projects/macos-ref/extract/ref27-index.tsv`,
`System/Library/ExtensionKit/Extensions/DisplaysExt.appex`), but its bundle was not measured with REA in this run.
The work came from the shell code: `springboard/src/shell/alert_sound.rs`, `src/media/system_sound.rs`,
`src/bin/shell.rs` (`Effect::Alarm`), `src/shell/services/display_services.rs`, `src/shell/night_shift.rs`,
`src/shell/audiosvc.rs`, `src/shell/services/audio_devices.rs`.

## Per area

| Area | State before | State after this slice |
| --- | --- | --- |
| Alert sound (Settings > Sound > Alert Sound, four tones) | Stored as `alert_sound`; `Effect::Alarm` only logged | Played. `alert_sound::pcm` synthesises each tone; `SoundPlayer::alert` plays it on the sound-effects role (audiod stream). A worker thread (`spawn_alert_worker`) owns the player so the 0.9 s write never blocks the UI. Silent while muted (zero volume), same gate as the clock. |
| Timer and alarm sound | Timers and alarms emitted `Effect::Alarm` | Same path: `Shell::alert_tone` returns the chosen index, or `None` while muted. |
| Key clicks (`Effect::KeyClick`) | Not played (binary ignores the effect) | Unchanged. `SoundPlayer::click` now exists on the same player, so wiring it is one arm in the binary. Left for the owner of keyboard sounds. |
| Output / input device switching and volume | Through `services::audio_devices` (`defaults`, `set_volume`) and `volume_worker` (audiod HAL) | Unchanged. Already goes through audiod. |
| Display resolution / refresh (`set_mode`) | Answers `pending`; the compositor owns the output mode | Unchanged. |
| Display scale, arrangement | Interface Scale page; one output, no arrangement page | Unchanged. |
| Night Shift warmth | Software tint of the final frame in the shell (`night_shift::tint`) | Unchanged. |

## Rows not closed in this slice

- **Resolution and refresh apply.** `bifrost-wm` (`wm/src/drm.rs`) makes the one modeset at boot (`choose_mode`,
  `BIFROST_WM_MODE`) and keeps the mode the CRTC already shows. Applying a mode live needs a request path from the shell
  (`display_services::set_mode`) into wm and a new `create_surface` on the CRTC. Neither exists, and a modeset cannot be
  checked without building and running on Midget. The shell cannot change the mode itself: it runs as a Wayland client
  under wm in a desktop session.
- **Night Shift through the gamma path.** There is no gamma ramp in wm or the output layer yet. `render/src/gamma_fade.rs`
  is the screen saver's fade model, not a gamma table. Moving warmth to a real gamma ramp needs the same wm output path
  as the mode apply.
- **Arrangement.** One output in the shell; no second display to arrange.

## Midget check (lead)

- Settings > Sound > Alert Sound: pick each tone, then set a timer for one second; the tone plays once and the
  shell logs no `alert sound` error.
- Set volume to zero and let a timer fire: the shell logs `alarm (muted)` and plays nothing.
- Confirm `audiod` shows the sound-effects stream when the alert plays.
