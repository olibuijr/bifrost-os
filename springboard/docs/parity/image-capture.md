# Image Capture and scanners (parity slug `image-capture`)

Covers the ImageCaptureCore rows and the Image Capture app and Print & Scan scanner list in macOS 27.
Work is on branch `worktree-wf_60b84374-fd9-11`, based on `wave-base` (44f22ddb).

## Evidence used

- Reference API names: taken from the matrix rows in `docs/parity-matrix/data/imagecapturecore.json`. The BridgeSupport
  inventory in the scratchpad was not consulted for this slice.
- Reference app bundle: `System/Applications/Image Capture.app` is in `~/Projects/macos-ref/extract/ref27-index.tsv`
  (Resources, Base.lproj/Main.storyboardc, en.lproj). The bundle itself is NOT extracted under
  `~/Projects/macos-ref/extract/ref27/` yet, so no REA decode of its storyboard was done. The window layout here is
  from the brief (sidebar, pictures, scan settings), not from the storyboard. Re-check once the lead extracts it.
- Settings extension `PrinterScannerSettings.appex` is in the index but not extracted; the Print & Scan scanner list
  is derived from behaviour (what the scanner command reports), not from its strings.
- Existing code read: `springboard/src/shell/camera/devices.rs` (DCIM volumes, `import_files`, `contents_of`),
  `springboard/ui/src/camera_device.rs`, `springboard/ui/src/device_browser.rs` (Scanner kind and mask),
  `springboard/ui/src/scan_job.rs` (functional units, settings, job gate, overview and scan requests),
  `springboard/ui/src/cups.rs` and `springboard/src/shell/print_scan_page.rs` (the Print & Scan pattern).

## Rows

| Matrix entry (data/imagecapturecore.json) | BifrOSt path | Done | Still needs Midget check |
|---|---|---|---|
| Scanner discovery and functional units (ICScannerDevice, availableFunctionalUnitTypes, requestSelect) | `ui/src/scanner.rs` (`parse_list`, `list`, `Capabilities::parse_help`, `Capabilities::unit`); `ui/src/scan_job.rs` (`ScannerDevice::select`) | `scanimage -L` lists scanners; `--help` gives resolutions and colour modes; the flatbed unit feeds `ScannerDevice` | a real SANE scanner (USB and network) and its `--help` output |
| Scanning (requestOverview, requestScan, cancelScan, transferMode, documentUTI, downloadsDirectory) | `ui/src/scanner.rs` (`scan_args`, `scan_to`, `scan_file_name`); `ui/src/scan_job.rs` (`request_overview`, `request_scan`, `finish_job`); `apps/image-capture-desktop/src/worker.rs` | Preview (75 dpi PNG to the cache) and Scan (to `~/<downloads_directory>`, named `<document> <time>.<ext>`); file-based transfer only; PDF refused | a scan end to end; cancel is not wired (no cancel button yet) |
| Device browser for scanners (ICDeviceBrowser, scanner half) | `ui/src/scanner.rs` (`device_entries`), `ui/src/device_browser.rs` | Scanners map to `Device` entries with `DeviceKind::Scanner`; the Image Capture app re-lists every 4 s | hotplug timing on real hardware |
| Print & Scan scanner list (PrinterScannerSettings, behaviour) | `src/shell/print_scan_page.rs` (`scanners_group`, `View::with_scanners`) | A SCANNERS group with each scanner's connection and a "No Scanners" row or the reason | the page on Midget |
| Image Capture app (os/apps `Image Capture.app`) | `apps/image-capture-desktop/` (manifest, model, worker, window, draw) | Sidebar of cameras and scanners; import of picked or all pictures into Pictures; scanner settings (resolution, colour, format), Preview and Scan | the app window on Midget, a host preview (`bifrost pc smoke`) |
| Camera file browsing and download (partial) | `apps/image-capture-desktop/src/model.rs` (`camera_files`); `springboard/src/shell/camera/devices.rs` (`contents_of`, `import_files`) | Pictures of DCIM volumes are listed and copied into Pictures | the copy on a real SD card; still no PTP or MTP |

Not closed by this work: PTP and MTP transport and camera control (ICCameraDevice tethering, delete, clock sync),
Protected devices (session login: SANE scanners here are unprotected), per-app USB device grant
(`com.apple.security.device.usb`), and the camera permission prompt. Cameras are DCIM volumes only (no gphoto2).

## Decisions

- The scan model is `ui/src/scan_job.rs`, not a second model. `ColorMode` (Colour, Greyscale, Black & White) and
  its labels were added there; `DocumentFormat` gained `label` and `next`; `ScanSettings` gained `color`.
  `ui/src/scanner.rs` is the SANE transport only (the command line), so no duplicate settings type exists.
- A scan goes through the job gate of `ScannerDevice` (`request_overview` / `request_scan` before the command runs,
  `finish_job` when the reply arrives). A refused request says why in the footer (`model::refusal`).
- PDF is in the format list of `scan_job`, but `scanimage` cannot write it; the app skips PDF when cycling and
  `scanner::scan_args` refuses it.
- The preview is always PNG (the window decodes it with the SDK image source) and lives in
  `~/.cache/image-capture/preview.png`.
- Scans are named with the time in UTC (`model::stamp`), because the shell has no time-zone library in the app.
- Imports go to `~/Pictures` (Photos scans that folder); only still images are copied, as `import_files` does.
- Print & Scan runs `scanimage -L` each time the page is built. A network scanner's discovery can take seconds on the
  shell thread; if that shows on Midget, move the listing to a worker.

## Shared-file edits

- `springboard/Cargo.toml`: `apps/image-capture-desktop` added to workspace members. The lead's build updates
  `Cargo.lock` for the new package.
- `springboard/ui/src/lib.rs`: `pub mod scanner;` after `pub mod cups;`.
- `springboard/src/shell/print_scan_page.rs`: View gains `scanners` and `scanner_error`; `groups` appends the
  SCANNERS group after the printers group; the no-printing-service test now expects two groups.

## Migrations and deletions

- None of the existing code was superseded. `scan_job.rs` (the settings and job model from theme 33) is the one model
  and was extended, not duplicated.

## Open

- `apps/image-capture-desktop/icon.png` is not in the tree (the manifest names it, as printers-desktop does).
- Cancel of a running scan, a document feeder unit (`--source`), and a choice of save folder are not in the window.
- `scanimage` is the only scanner transport; the program must be installed on the system.
