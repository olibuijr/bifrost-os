# Recording polish (parity round 75)

Slice: the screen recording follow-up. Frames at the display's rate where the GPU path allows, an optional microphone
take from the audio service, the menu bar stop item, finished recordings opened in the Video Player, and the screenshot
toolbar's Options menu (save location, timer, pointer). The ffmpeg H.264 path and the built-in Motion JPEG AVI writer
both stay.

## What is built

| Part | BifrOSt path |
|---|---|
| Frame rate: the display's refresh rate (10 to 60 frames a second) while the GPU path is up, else 10 | `springboard/src/shell/screenrec.rs` (`frame_rate`, `Shell::set_display`, `Session::start`); `springboard/src/bin/shell.rs` (the loop calls `set_display` each pass) |
| Per-session frame interval (was a fixed 10 fps constant) | `screenrec.rs` (`Session::interval`, `Session::fps`); the AVI header and ffmpeg `-r` take the session rate |
| Optional microphone take: the audio service records a WAV beside the movie (`RecordStart`, `RecordStop`), then ffmpeg copies the video and adds the take as AAC | `screenrec.rs` (`Shell::recording_audio`, `recording_done`, `finish_mux`, `mux_audio`, `mux_args`, `Audio`, `PendingMux`) |
| Microphone permission: the shell asks once through the Allow / Don't Allow sheet (the privacy service, under the shell's own id) and records sound only when allowed | `screenrec.rs` (`ask_microphone`, `recording_audio`); `services/privacy_permissions.rs` (`queue_prompt`, `access`) |
| Stop item in the menu bar: red item beside Control Center, its menu has Stop Recording (built earlier, unchanged) | `springboard/src/shell/desktop_bar.rs` (`Menu::Recording`, `Action::RecordStop`) |
| Finished recordings open in the Video Player: the player is started if needed and asked for the movie through its `open-movie` intent, after the take is muxed in | `screenrec.rs` (`open_recording`, `recording_tick`); `springboard/apps/videoplayer-desktop/src/main.rs` (`intents`, `perform_intent`), `manifest.toml` (`[[intent]]`) |
| Toolbar Options menu above the bar: Save To (Desktop or Documents), Timer (Off, 5 s, 10 s), Show Pointer, Microphone. The Timer and Pointer toolbar buttons are replaced by one Options button | `springboard/src/shell/capture.rs` (`Tool::Options`, `Prefs`, `OptionRow`, `Session::menu_rows`, `option_at`, `on_menu`); `shell/mod.rs` (`capture_touch`, `finish_capture`, `capture_request`); `shell/hwkeys.rs` (Shift-Cmd-5) |
| Options persist across capture sessions: the shell keeps `Prefs`, and each session starts from it | `shell/mod.rs` (`capture_prefs`); `capture.rs` (`Session::with_prefs`) |
| Fallback: without ffmpeg on the PATH the built-in Motion JPEG AVI writer records, and that recording has no sound | `screenrec.rs` (`choose`, `Encoder::Avi`); no audio is started for it |

## Decisions

- The frame rate is chosen when a recording starts and stays fixed for it. The software frame is drawn in full, so 10
  frames a second stays the rate off the GPU path. On the GPU path the rate is the display's, capped at 60.
- The microphone take is muxed after the audio service writes the WAV (its RecordDone event). If the take has not arrived
  within 15 seconds, the movie opens without sound and the take file is removed.
- The take is a hidden `.<name>.audio.wav` beside the movie.
- Save To applies to recordings. Still pictures keep going to Photos, as before.
- The Video Player's open goes through its intent, not a document grant: the player reads the file by path. Whether its
  sandbox allows reading the Desktop and Documents folders is the first thing to check on Midget.

## Gaps

- The pointer is not drawn into a movie (only into pictures). The shell does not track the pointer outside a capture
  session.
- Save To does not apply to pictures (see Decisions).
- The Video Player's own "New Screen Recording" menu item is still off: it does not reach the shell's recorder.
- `springboard/apps/screenshot-desktop` still has its own options model (`options.rs`: target, mode, destination, timer,
  pointer) that duplicates `capture::Prefs`. Nothing in the shell calls it. A later round should delete it or make the app
  read the shell's choices.
- A microphone that was never allowed, or an audio service that is not running, records a silent movie with a log line.
  There is no on-screen notice.

## Evidence

- Code only, not built or run (the lead builds). Unit tests: `screenrec.rs` (`frame_rate`, the 60 fps session timing and
  header, the mux arguments, the ffmpeg arguments, the AVI layout), `capture.rs` (the Options rows, prefs carried into a
  session, the Options toggle).
- No REA or reference-file evidence was measured this round. The behaviour follows the existing shell recording and the
  audio service protocol (`proto/src/svc.rs`: `RecordStart`, `RecordStop`, `RecordDone`).

## Still needs Midget checking

- Shift-Cmd-5, Record Screen: the Options menu opens above the bar; Save To, Timer, Show Pointer and Microphone change
  their text; the menu closes on a press outside it.
- A GPU-path recording runs at the display rate (`ffprobe` on the movie shows the frame rate), and a software one at 10.
- Microphone on: the Allow sheet appears once; a recording then carries an AAC track that plays on the TV and on Midget.
- The stop item, then the movie opens in the Video Player. Check that the player can read the file from the Desktop.
