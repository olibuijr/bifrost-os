# Connecting Voice Memos to the system services (1.3.0)

App: `springboard/apps/voicememos-desktop/` (`bifrost-voicememos`, version 1.3.0, unchanged this round).
Branch `connect/voicememos`. No builds were run (the lead builds once); the host tests listed below are written but
not yet run.

## What is connected

| Service (SDK client) | Matrix slug (`docs/parity-matrix/data/`) | What the app does | Where |
|---|---|---|---|
| `privacy` (`privacy_permissions`) | `avfoundation-capture-audio` | Asks the microphone status on start and on every Record press. Record is refused when the status is Denied or Restricted, with a message. A status that turns Denied during a recording stops it. A line under the record button shows the state. | `src/services.rs` (`record_gate`, `caption`), `src/voicememos.rs` (`start_recording`, `reply`) |
| `audio-devices` (`audio_devices`) | `core-audio` | Asks the device list and defaults on start, and again on every `devices_changed` or `route_changed` event. Shows the default input's name. Says "Input changed to X" when the default input changes while recording. | `src/services.rs` (`Want::ListDevices`), `src/voicememos.rs` (`set_input`, `devices_changed`), `src/main.rs` (`service_event`) |
| `now_playing` (`now_playing`) | `media-player` | Publishes the playing recording (title, length, playing or paused, playhead) to the shell's media tile, and withdraws it when playback ends, the app suspends, or the memo is renamed, trimmed or deleted. Handles the tile's play, pause, toggle, stop, skip forward, skip backward and seek commands. Next and previous are not declared. | `src/voicememos.rs` (`new_session`, `content_for`, `publish_playback`, `end_playback`, `command`, `seek_to`), `src/main.rs` (`service_event`) |

Plumbing:

- The model never calls a service. It queues a `services::Want` on a real event. `main.rs` sends the queue after each
  event, tick, configure, suspend, reply or service event (`flush`).
- `services::Bus` keeps the request ids of the two questions (microphone status, device list) so that
  `App::service_reply` knows which reply is which. Publish and withdraw need no reply. Unrelated replies are left for
  `service::take_reply`.
- A missing service (an older shell answers "no service ...") gives an error reply. The microphone stays unknown
  (recording is allowed as before), the input name is cleared, and the tile is not shown. The app keeps working.
- Nothing is polled. Services are called only on Record, playback start and stop, seek and skip, a device or route
  event, and a reply to a question the app asked.

## How a user sees it

1. **Microphone**: open Voice Memos. The line above the record button reads `Microphone on. Input: <device>` when the
   permission is allowed. Turn the permission off in Settings > Privacy > App Permissions, then press Record: the line
   above the button says why, and nothing is recorded. Press Record again after allowing it: a new recording starts.
2. **Input device**: the same line names the default input. Plug in or choose another input while recording: the line
   reads `Input changed to <device>` until the next Record press.
3. **Now Playing**: select a recording and press Play. The shell's media tile shows its title and length. Use the tile's
   pause to stop (the app's Play button toggles the same way), its skip buttons for 15 seconds, or its seek slider. The
   tile disappears when playback finishes or the app closes.

## How to verify on the test PC

Order: host tests first, then Midget (`bifrost pc midget ota` until `phase=accepted`). Skip the VM.

1. Host: `cargo test -p bifrost-voicememos` (lead). The new tests are:
   - `services::tests::a_mic_want_calls_the_privacy_service_and_its_reply_is_the_mic_question`
   - `services::tests::a_device_want_calls_audio_devices_and_publish_and_withdraw_need_no_reply`
   - `services::tests::an_unknown_request_is_not_an_answer`
   - `services::tests::a_denied_or_restricted_microphone_refuses_the_recording_and_the_rest_allows_it`
   - `services::tests::the_caption_shows_the_microphone_state_and_the_input_name`
   - `voicememos::tests::a_new_app_asks_for_the_microphone_and_the_devices_and_opens_no_device`
   - `voicememos::tests::a_denied_microphone_refuses_to_record_and_the_app_asks_again`
   - `voicememos::tests::a_missing_privacy_service_leaves_the_microphone_unknown_and_recording_allowed`
   - `voicememos::tests::the_input_device_is_named_and_a_change_during_a_recording_is_noted`
   - `voicememos::tests::a_route_or_device_change_asks_for_the_device_list_again`
   - `voicememos::tests::a_playing_recording_is_published_and_withdrawn_when_playback_ends`
   - `voicememos::tests::a_shell_seek_or_skip_moves_the_playhead_within_the_recording`
2. Midget, microphone: open the app with
   `bifrost pc midget open-app is.olibuijr.voicememos.desktop`, then `bifrost pc midget screen /tmp/vm-mic.png`. The
   caption reads `Microphone on. Input: ...` (or `not answered yet` on a fresh install).
3. Midget, denial: in Settings > Privacy > App Permissions set Voice Memos' microphone to off. Press Record (use
   `bifrost pc midget tap-window APP X Y` with window-local pixels). The red line says Microphone access is off, and no
   new file appears in the list.
4. Midget, now playing: record a short take, select it, press Play. `bifrost pc midget state` should show the media
   tile; take a screenshot. Use the tile's pause; the tile's state reads paused. Press Play again; the tile reads playing.
5. Midget, input change: with a second input connected, select it in Settings > Sound while recording. The caption
   reads `Input changed to ...`.

## Gaps and notes

- **Matrix rows**: `docs/parity-matrix/data/` is not edited in this round (outside the app folder). Rows move to Partial
  after the Midget checks above, and the lead updates them (`avfoundation-capture-audio`, `core-audio`, `media-player`).
- **Pause**: the app has no pause, so the tile's pause stops playback and the playhead returns to the start, as the
  Play button already does. A real resume position needs an audio service pause command.
- **Permission prompt**: the privacy service only answers. The shell does not prompt from these calls, so a fresh install
  reads "not answered yet" and records as before until the person answers in Settings.
- **Permission check is one press behind**: the gate uses the last answer. Each Record press asks for a fresh answer,
  so a permission changed in Settings is seen on the next press (a recording already running stops when a reply says the
  permission was turned off).
- **Output route**: `route_changed` only refreshes the device list. The app does not show the output device.
- **Artwork and per-device metering**: the media tile has no artwork (the Now Playing artwork path does not reach the
  shell yet), and the audio service reports one linear level, not dB per channel.
- **Shell version**: the three services are registered in the shell tree at this commit. `min_shell` in
  `manifest.toml` is still `0.4.84`, because the app's version and manifest are not changed this round. An older shell
  answers "no service ..." and the app runs without the features; the lead may raise `min_shell` at release.
- **Official pages**: the behaviour follows the SDK notes and the matrix rows named above. No new REA evidence was taken
  this round.
