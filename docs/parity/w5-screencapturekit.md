# ScreenCaptureKit parity (wave 5)

Branch `parity/w5-screencapturekit`, on top of `parity/integration` (waves 1 to 4). Pure Rust, std only, no new
crates, no unsafe, no hardware or network access. No build was run in this round: the tests were checked by hand
against the rules the code implements and have not been compiled or run.

## What was built

1. **System content-sharing picker model** (`springboard/ui/src/capture_picker.rs`, new). A `Picker` that the shell
   presents and an app configures. It holds the five selection modes, the choice the person made (`Selection`), and
   the app's rules: allowed modes, excluded windows and applications, and whether a made selection may be changed.
   `choose` checks the mode, the item count (one for single modes, at least one for multiple modes), the kind of each
   item, duplicates, and the exclusion lists before it accepts a choice. Errors: `NotPresented`, `ModeNotAllowed`,
   `WrongContent`, `CountMismatch`, `Duplicate`, `Excluded`, `SelectionLocked`.
2. **Screen-recording permission** (`springboard/ui/src/capture_access.rs`, extended). The existing per-app
   permission model had camera and microphone only. A third media type, `Media::Screen`, now has its own status with
   the same rules: asked once, changed through Settings, and blocked by a system restriction. The camera and microphone
   slots are unchanged.
3. **Single-frame screenshot settings** (`springboard/ui/src/capture_shot.rs`, new). `ShotConfig` holds the output
   width and height, the image format (PNG or JPEG), the quality of a lossy format, the pointer and shadow options, and
   an optional source area. `plan(display_w, display_h)` checks the settings against the display and returns the
   captured area and the output size. Errors: `InvalidSize`, `InvalidQuality`, `EmptySource`, `SourceOutside`.

Not built: no SDK module in `springboard/sdk/src/` calls the shell's capture path, so no app can take a screenshot
through this model yet. The shell's screenshot chord and the agent tooling's capture are unchanged. The
persistent-capture part of the permission row (the Boolean for a remote-desktop app that needs ongoing access, and its
privacy string) is not built.

## Official API names mirrored

Taken from the rendered official ScreenCaptureKit page and the framework's DocC index (`python3 -I tools/parity/docc.py
screencapturekit`). The DocC page for the picker configuration struct returned 404, so its property names are not
mirrored; only its description and the mode cases are confirmed.

- System content-sharing picker: `SCContentSharingPicker`, `SCContentSharingPickerConfiguration`,
  `SCContentSharingPickerMode` (cases `singleDisplay`, `singleWindow`, `singleApplication`, `multipleWindows`,
  `multipleApplications`), `SCContentSharingPickerObserver`. Source: the picker mode page
  https://developer.apple.com/documentation/screencapturekit/sccontentsharingpickermode
- Screen recording permission: the Persistent Content Capture symbol and the screen-recording prompt, described on the
  framework page https://developer.apple.com/documentation/screencapturekit.
- Single-frame screenshots: `SCScreenshotManager`, `SCScreenshotConfiguration` (properties `width`, `height`,
  `contentType`, `sourceRect`, `destinationRect`, `showsCursor`, `ignoreShadows`), `SCScreenshotOutput`. Source: the
  configuration page https://developer.apple.com/documentation/screencapturekit/scscreenshotconfiguration

## REA evidence

- Framework landing page: `ev_141c84c6a16b3cea5a5979b052828e4fab1a6f0c942267c755692144a7a68754`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/screencapturekit`). It confirms the picker as
  the recommended selection UI and the screen-recording permission request.

## Matrix rows closed

| Framework | Feature (as in data/screencapturekit.json) | New status | BifrOSt path |
|---|---|---|---|
| ScreenCaptureKit | System content-sharing picker (SCContentSharingPicker, SCContentSharingPickerConfiguration, SCContentSharingPickerMode, SCContentSharingPickerObserver) | partial | springboard/ui/src/capture_picker.rs |
| ScreenCaptureKit | Permission for screen recording and persistent capture (Persistent Content Capture, privacy string) | partial | springboard/ui/src/capture_access.rs |
| ScreenCaptureKit | Single-frame screenshots (SCScreenshotManager, SCScreenshotConfiguration, SCScreenshotOutput) | partial | springboard/ui/src/capture_shot.rs |

The single-frame row was an `os` row. It stays partial, not parity: the pure model exists but no app-facing SDK call
uses it yet, and the row needs that wiring to count as covered.

## Owner rules

`python3 tools/check-names.py` reports no hits. The code uses no Apple platform or product names in identifiers or
comments.
