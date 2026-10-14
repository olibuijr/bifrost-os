# Parity sdk-os-09: app surfaces over system capabilities

The matrix rows in this task were `os`: the shell or a daemon did the work and apps had no SDK type for it. This note
lists the app-facing surface each row now cites, what is new code, what is existing code that the row now points to, and
what still needs the lab machine.

## New models (ui crate, reachable from apps as `bifrost_app::controls::<module>`)

| Module | Mirrors | What it does | Tests |
|---|---|---|---|
| `springboard/ui/src/capture_session.rs` | the capture session model | Holds one usable video input and a frame size. `start` plans the stream with `video_device::plan_stream`. Input and size are locked while running. | `ui/tests/parity_sdk-os-09.rs` |
| `springboard/ui/src/photo_capture.rs` | the photo settings model | Checks the image format, JPEG quality, self-timer delay (0, 3 or 10 seconds) and square crop. `plan` returns the crop of a frame. | `ui/tests/parity_sdk-os-09.rs` |
| `springboard/ui/src/control_widget.rs` | control widgets | A registry of buttons and toggles with unique identifiers, display names, toggle values and presses. | `ui/tests/parity_sdk-os-09.rs` |

None of the three opens a device, draws a frame or reads the clock.

## Existing models that the rows now cite

Each of these already has its own model and inline tests. The matrix rows point to them with their gaps.

- Audio sessions and routes: `sdk/src/audio_session.rs`, `sdk/src/audio_devices.rs`.
- Audio formats and conversion: `sdk/src/audio_format.rs`, `ui/src/pcm_convert.rs`.
- Bluetooth pairing, devices and BR/EDR class: `sdk/src/bluetooth_service.rs`, `sdk/src/bluetooth_devices.rs`.
- Accessories and devices: `sdk/src/accessory_access.rs`, `sdk/src/external_accessory.rs`, `sdk/src/hid_devices.rs`,
  `ui/src/device_browser.rs`, `ui/src/device_match.rs`, `ui/src/video_device.rs`.
- Now Playing: `sdk/src/now_playing.rs`, `ui/src/media_session.rs`, `ui/src/playback_arbiter.rs`.
- Playback: `ui/src/av_player.rs`, `ui/src/av_queue_player.rs`, `sdk/src/audio_player_control.rs`.
- PDF: `sdk/src/graphics_pdf.rs`, `ui/src/pdfpage.rs`.
- Drawing: `sdk/src/gpu_layer.rs`, `ui/src/drawable_layer.rs`.

## Still needs the lab machine

- Capture: the shell's Camera app runs its own V4L2 path. The session model is not yet the path the Camera app uses,
  so the frame plan has to be checked against a real camera on Midget.
- Photos: JPEG writing and EXIF stay in the Camera app. No app-facing photo output takes a picture yet.
- Audio sessions: `audiod` does not arbitrate sessions yet. Interruptions and route changes are modelled but not
  delivered from the audio service.
- Bluetooth: pairing and forgetting run in the shell. Bond keys stay in `bluetooth.ini`, and no app reads them.
- Control widgets: Control Center shows its fixed toggles and does not read the registry yet.
- Now Playing: the shell shows the newest session. The arbiter is not yet applied to that choice.
- Video decoding: the decoder seam (`springboard/src/media/mod.rs`, `hwdec/`) is a shell-side trait over libavcodec and
  VA-API. No app-facing decode surface exists, and the row stays `os`.
