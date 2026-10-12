# Parity wave 4: ScreenCaptureKit models

Four pure models in `springboard/ui/src/` (`bifrost_ui`) that cover the data side of the official screen capture
framework: what can be captured, what a filter takes, how a stream is configured and which error codes it ends with.
No capture runs. There is no frame stream, no compositor call and no app-facing SDK API yet; those need a capture
pipeline that the shell does not offer to apps.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/capture_content.rs` | SCShareableContent, SCDisplay, SCRunningApplication, SCWindow | Holds displays, apps and windows (front to back). Queries with on-screen-only, desktop exclusion and above or below a window. Finds the display holding most of a window. |
| `springboard/ui/src/capture_filter.rs` | SCContentFilter (the creation forms) | One window, or one display with a rule: include or exclude windows, include or exclude apps with listed window exceptions. Resolves against the shareable content. |
| `springboard/ui/src/capture_config.rs` | SCStreamConfiguration (dimensions, scaling, queue depth, frame interval, cursor, audio) | Validates the configuration, gives the frame rate from the interval, and computes where a source lands in the output. |
| `springboard/ui/src/capture_error.rs` | SCStreamError and SCStreamError.Code | The 21 error codes in the reference's five groups, with a group name, a short text, and the user-cancellation and privacy checks. |

Each module has a `#[cfg(test)] mod tests` with hand-worked expected values.

## Rules we chose (not stated by the reference)

- Filter exceptions: a listed window counts as outside its app. For an include-apps rule the listed window is left
  out; for an exclude-apps rule the listed window is kept. The reference gives only one-line summaries of the
  "excepting" initialisers.
- Stream configuration defaults (1920 by 1080, queue depth 3, 60 frames per second, cursor shown) are ours.
- Queue depth is limited to 1 to 8 (`MAX_QUEUE_DEPTH`). This is a BifrOSt bound, not a value from the reference.
- Without scaling, the source sits at the top left and is cut to the output size.
- The error texts are our own wording for each code.

## Official references

- https://developer.apple.com/documentation/screencapturekit
- https://developer.apple.com/documentation/screencapturekit/sccontentfilter (creation forms, filter properties)
- https://developer.apple.com/documentation/screencapturekit/scstreamconfiguration (queueDepth, minimumFrameInterval, scalesToFit, showsCursor, audio)
- https://developer.apple.com/documentation/screencapturekit/scstreamerror/code (SCStreamError.Code groups)
- https://developer.apple.com/documentation/screencapturekit/scshareablecontent
- https://developer.apple.com/documentation/screencapturekit/scwindow

## REA evidence

- `ev_bd2c798f4ca9a4b9599cc70c52924d3102164e5445d9735024c5b27ec6eaa9a9`: rendered framework landing page.
- `ev_df3adf6ae124155193aea865d579c53db7df64b3e7e297485bdd46a1cb1f452d`: rendered SCContentFilter page (creation forms and filter properties).

## Matrix rows closed (status partial)

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| ScreenCaptureKit | Discover displays, apps and windows to capture (SCShareableContent, SCDisplay, SCRunningApplication, SCWindow) | partial | springboard/ui/src/capture_content.rs |
| ScreenCaptureKit | Filter what a stream captures (SCContentFilter) | partial | springboard/ui/src/capture_filter.rs |
| ScreenCaptureKit | Stream screen content to an app (SCStream, SCStreamConfiguration, SCStreamOutput) | partial | springboard/ui/src/capture_config.rs |
| ScreenCaptureKit | Stream errors and lifecycle (SCStreamDelegate, SCStreamError) | partial | springboard/ui/src/capture_error.rs |

Still open: the frame stream and its output delegate, the content-sharing picker, the screen recording permission flow,
the screenshot configuration for apps, and clip buffering.
