# AVKit (parity/team-k-avkit)

Round for the AVKit framework rows in `docs/parity-matrix/data/avkit.json`. Built on branch `parity/team-k-avkit`
(from `arm`). The work is pure Rust state in `springboard/ui/src/`, with host unit tests written but not run in this
round (the round rule is no builds). Nothing here is wired into the SDK, the shell or an app, and nothing is verified
on Midget yet.

## Gap rows: now partial

| Row | Module | What the model does | Not done |
| --- | --- | --- | --- |
| Display management (HDR, refresh mode switching) | `springboard/ui/src/display_manager.rs` | Criteria a playing app asks for (refresh rate within a tolerance, dynamic range), choice of a mode that meets them, and a switch-in-progress flag. Nothing is switched in the model. | Not in the SDK; the shell display list performs the switch (`docs/parity/displays-modeset.md`); HDR not on hardware. |
| Route selection (AirPlay and nearby receivers) | `springboard/ui/src/route_picker.rs`, `springboard/ui/src/route_picker_view.rs` | Destination rows (local outputs first, then receivers by name, active flag from `route_auth`); button look (System, Inactive, Active) and the will-begin and did-end presenting events. | No receiver discovery feeds the rows; nothing draws the picker; no route is chosen from it. |

## Partial rows deepened

| Row | Module | What it adds |
| --- | --- | --- |
| Capture view and camera viewfinder (AVCaptureView) | `springboard/ui/src/capture_view.rs` | Session binding with video and audio preview flags (clearing it hides both previews), controls style (Default, Inline, Floating, InlineDeviceSelection), video gravity with its layer raw values, and `request_recording`, which returns the start-recording event or a refusal (no session, empty path). |
| Camera facing direction (AVCaptureDeviceDirectionCoordinator, Map, DeviceDescriptor) | `springboard/ui/src/capture_direction.rs` | Device descriptors (unique id, name, device type, media types, position) grouped into forward and backward lists relative to the view's side, unspecified cameras left out, a device-type filter, identifier-sorted lists, and a change report only when the grouping differs. |

## Official API names mirrored

Read with `python3 -I tools/parity/docc.py`:

- `AVCaptureView` (macOS 10.10): `setSession(_:showVideoPreview:showAudioPreview:)`, `session`, `controlsStyle`,
  `videoGravity`, `delegate`. `AVCaptureViewDelegate`: `captureView(_:startRecordingTo:)`.
  `AVCaptureViewControlsStyle`: `default`, `inline`, `floating`, `inlineDeviceSelection`.
  https://developer.apple.com/documentation/avkit/avcaptureview
- `AVCaptureDeviceDirectionCoordinator` (`init(view:deviceTypes:changeHandler:)`, `deviceDirections`),
  `AVCaptureDeviceDirectionMap` (`forwardFacingDeviceDescriptors`, `backwardFacingDeviceDescriptors`),
  `AVCaptureDeviceDescriptor` (`uniqueID`, `localizedName`, `deviceType`, `mediaTypes`, `position`). The pages list
  these as Mac Catalyst 27.1 only; the Rust names follow them and the model is host-only.
  https://developer.apple.com/documentation/avkit/avcapturedevicedirectioncoordinator

Not mirrored: the view's drawing, the `delegate` object itself (events are returned, not sent), the file output that
records, and `init(view:...)` taking a view (the model takes the view's side instead).

## Not done and why

- No SDK surface exposes these models yet. Wiring them needs the shell's capture service and the display list, which
  are shared with other rounds, so this round stays in the models.
- Midget check is pending for every row above; a row stays `partial` until it works on Midget.
- The capture events and audio input picker rows keep their existing status. Their models are from wave 5 (see
  `docs/parity/w5-avkit.md`).
