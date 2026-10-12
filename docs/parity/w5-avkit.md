# Wave 5: AVKit (parity/w5-avkit)

Two pure models in `springboard/ui/src/` for AVKit rows that were gaps or OS-only. Both are pure Rust with host
unit tests, and neither is wired into the SDK, the shell or any app yet. They were not built or run in this wave
(the wave rule is no builds), so the tests are written but not executed.

## What was built

- `springboard/ui/src/capture_event.rs`: hardware capture button registration. An `Interaction` is created either
  with one shared handler (`single`) or with separate primary and secondary handlers (`split`). `dispatch` decides
  where a press goes (`Handler(Slot)`) or returns it to the system (`System`) when the interaction is disabled, the
  app is not capturing, or the app is in the background. `is_completed_press` is true only for the ended phase.
  Phases are began, ended and cancelled.
- `springboard/ui/src/input_picker.rs`: audio input picker state. `offered_inputs` returns an empty list unless the
  session records. `Picker` tracks presentation (`present` and `dismiss` are idempotent and queue `WillPresent`,
  `DidPresent`, `WillDismiss`, `DidDismiss`), the chosen input (`select` accepts only an offered id) and falls back
  to `None` when the chosen input has vanished (`selected_in`).

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `AVCaptureEventInteraction` (`init(handler:)`, `init(primary:secondary:)`, `isEnabled`,
  `defaultCaptureSoundDisabled`), `AVCaptureEvent`, `AVCaptureEventPhase` (`began`, `ended`, `cancelled`).
  Source: https://developer.apple.com/documentation/avkit/avcaptureeventinteraction
- `AVInputPickerInteraction` (`init()`, `init(audioSession:)`, `isPresented`, `present()`, `dismiss()`,
  `delegate`). Source: https://developer.apple.com/documentation/avkit/avinputpickerinteraction

Behaviour taken from the rendered pages: the system sends capture events only to apps that actively use the
camera, not to backgrounded apps; disabling the interaction restores system button behaviour; a session in any
mode other than recording produces an empty input list; the picker reports transitions around its presentation.

## REA evidence

- `ev_de113459c8fdf115845520a92c61c535e3f83a9ca6935abef95dc40c4d8807b1` (AVCaptureEventInteraction page)
- `ev_298ee0030226cdd3c686a00e38c68fe1015711fe1d37e92f977d006db71bab59` (AVInputPickerInteraction page)

## Matrix rows closed

Rows are from `docs/parity-matrix/data/avkit.json`. Status is the new status for the row.

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| AVKit | Capture events from hardware buttons | partial | springboard/ui/src/capture_event.rs |
| AVKit | Audio input picker | partial | springboard/ui/src/input_picker.rs |

## Not done in this wave

- Media trimming and export (AVKit, gap) was skipped. `springboard/ui/src/recording.rs` already has a `Trim`
  range model, so a second one would duplicate it.
- No SDK API wraps either model. The audio input picker row stays `os` in substance until the audio service
  feeds the input list and an SDK type exposes `Picker`. The audio session object is not modelled.
- No hardware button reader feeds `capture_event` events. The input layer must do that.
