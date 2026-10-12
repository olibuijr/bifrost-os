# Connect: DAW app to the system services (`connect/daw`)

Scope: `springboard/apps/daw/` only (plus this note and the CHANGELOG entry). The app is `bifrost-daw` 0.2.6; the
version was not bumped this round. Not built or run here: the lead builds once.

## What is connected

| Service (shell, `springboard/src/shell/services/`) | Matrix framework slug | What the DAW does | Code |
|---|---|---|---|
| `audio-devices` | `core-audio` (output routing, route indicator) | Asks for the device list at start (`list`). Shows the route in the status line on `route_changed` ("Output: Headphones") and asks for a fresh list on `route_changed` and `devices_changed`. The default output name is kept from the reply. | `src/system.rs` (`System::start`, `reply`, `event`, `output_name`) |
| `now_playing` | `now-playing` (playback state, play, pause, stop) | While a song plays or is paused, Control Center shows a session with the project name and the play, pause and stop commands. The tile's commands drive the DAW: play resumes or starts, pause keeps the position, stop stops. The session is withdrawn on stop. Publishes only when the transport changes. | `src/system.rs` (`sync_session`, `session_for`), `src/main.rs` (`transport`, `pause`, `command`) |
| `power` | `foundation-networking-lowlevel` (ProcessInfo Low Power Mode) | Reads Low Power Mode at start (`lowPowerMode`). Changes come as `powerStateDidChange`. While it is on, a warning line shows at the bottom of the window: "Low Power Mode is on. Playback and rendering may be slower." | `src/system.rs`, `src/main.rs` (`draw`) |

Service calls are made only at start (once), on a route or device change, on a Low Power Mode change (no call, the
event carries the value), and when the transport changes. Nothing is sent per frame.

A shell that does not have a service answers with an error ("no service ..."). The reply is taken and the state
stays at its default (no output name, Low Power Mode off, no session). The app does not break.

## How a user sees it

- **Output:** change the output (plug in headphones, or unplug them). The status line at the bottom reads
  "Output: Headphones" or "Output: Speaker". Playback continues on the new route.
- **Now Playing:** press Play in a project. Swipe down from the top-right to open Control Center: the Now Playing tile
  shows the project name and the DAW's transport. Its pause button pauses the song and its play button resumes it from
  the same place. Stop in the app (or the tile's stop) removes the tile.
- **Low Power Mode:** turn it on in Settings or the battery menu. A warning line appears at the bottom of the DAW window
  and disappears when Low Power Mode is turned off. Nothing else changes (no render or playback limits are applied).

## How to verify on the test PC (Midget)

1. `bifrost pc midget ota` until `phase=accepted`, then open the DAW and take a screenshot.
2. Audio route: with the headset out, open the DAW and play a project. Plug in headphones: the status line reads
   "Output: Headphones". Unplug: "Output: Speaker". The song keeps playing.
3. Now Playing: play a project. Open Control Center: the tile shows the project name and a pause control. Tap pause
   with `bifrost pc midget tap-window` on the tile; the song pauses. Tap play; it resumes. Tap stop; the tile goes away.
4. Low Power Mode: `cat /run/bifrost/power-state` shows `low_power=0`. Turn Low Power Mode on in Settings: the DAW
   window shows the orange warning line within a moment (the event is pushed; no restart needed). Turn it off: the line
   goes away. A DAW opened while Low Power Mode is on shows the line at start.
5. Check the warning and the status line against the bifrost-hig checklist (colour from `ui::SYSTEM_ORANGE`, type from
   `MAC_CALLOUT_PT`).

Host tests (lead, once): `cargo test -p bifrost-daw system` (in `springboard/`), 12 tests in `src/system.rs`: service
call bodies and methods, replies (including errors from an older shell), route and device events, power reply and event,
tile commands, publish-once and withdraw-on-stop, session state and command enablement.

## Gaps

- **Sample rate:** the `audio-devices` service reports devices, routes and volume, not the output sample rate. The DAW
  renders at its own rate (`render::RenderOpts`) and cannot show or match the device rate. Not connected.
- **Route choice:** the app cannot pick the output (no `set_route` in the service); it only shows the route and the
  default-device choice is left to the shell.
- **Now Playing seek and skip:** the DAW does not declare seek, next or previous, so the tile never sends them. The
  playhead does not follow the tile's position; the session position is the value at publish time.
- **Artwork and duration:** the session has no artwork and a continuous duration (a loop has no end).
- **Lock screen:** the session shows in Control Center. The lock card still shows the audio service track.
- **Power:** the warning is shown only; the DAW does not reduce render quality or change the render cache while in Low
  Power Mode. Thermal state and battery (`power` service) are not used.
- **Dependency:** `springboard/apps/daw/Cargo.toml` gains `bifrost-core` (a path dependency already in the workspace,
  needed to name the `Json` type in the service hooks). `Cargo.lock` gets the new dependency edge on the lead's build.
- **Tests need the UI font:** like the SDK's own service tests, `src/system.rs` builds a `Ctx` from
  `springboard/assets/SFUI.ttf`. That file is not in the checkout, so these tests need it present to run.
- **Names:** `python3 tools/check-names.py` passes (no platform or product names in the new code).
