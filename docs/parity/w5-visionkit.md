# Wave 5: VisionKit parity (barcode scanning, stabilised camera region)

Branch `parity/w5-visionkit`, on top of `parity/integration` (waves 1 to 4). Pure Rust, std only, no new crates,
no unsafe, no hardware access. No builds or tests were run for this wave (wave rule).

## What was built

### 1. App-facing barcode and QR scanning (`springboard/sdk/src/data_scanner.rs`)

Closes the `os` row "Barcode and QR scanning through the camera". The QR decoder
(`springboard/src/shell/camera/qr.rs`, `decode_luma`) existed only inside the system Camera app. The SDK
(`bifrost_app`) already depends on the `springboard` library, and `springboard::shell::camera::qr` is public,
so the SDK now wraps it.

- `DataScanner::scan_luma(luma, w, h) -> Changes`: decodes one 8-bit luminance frame and reports the added, updated
  and removed items since the previous frame, through `bifrost_ui::scan::ScanSession` (the same delta a scanner
  delegate receives).
- `DataScanner::items()` and `DataScanner::stop()`.
- The item id is FNV-1a 64 of the payload, so one code keeps its id across frames. The hash is fixed (not `std`'s
  default hasher, which may change between releases).
- Re-exports: `Changes`, `Item`, `ItemKind`, `Point`, `Quad`, `ScanSession`, and `Stabilizer`/`DEFAULT_DEAD_BAND`.

Limits: QR model 2 only, one code per frame, no perspective correction, no text items, no zoom and no
recognized-data-type set. The outline of an item is the whole frame, because the decoder does not return the
code's position. Text recognition (OCR) is not covered by this row.

### 2. Stabilised camera region of interest (`springboard/ui/src/camera_region.rs`)

Closes the `gap` row "Stabilised camera region of interest". This is a pure model, not a view. There is no
camera feed or renderer in the shell for apps to draw into.

- `Stabilizer::new(frame)` and `Stabilizer::with_dead_band(frame, band)`, default band `DEFAULT_DEAD_BAND` = 4 pt.
- `observe(target) -> Rect`: clamps the target into the frame (cuts oversize regions to the frame size, then pulls
  the position inside). A move whose largest edge change is below the band is ignored. A larger move closes half
  the remaining distance per frame.
- `set_frame`, `current`, `reset`.

## Official API names mirrored

- `VisionKit` / `DataScannerViewController`, `DataScannerViewController.RecognizedDataType`,
  `DataScannerViewControllerDelegate` (barcode items, the `recognizedItems` stream): mirrored by `DataScanner` and
  the `Changes`/`Item` delta.
  Source: https://developer.apple.com/documentation/visionkit/datascannerviewcontroller
- `VisionKit` / `CameraRegionView` (stabilised region of interest): mirrored by `Stabilizer`.
  Source: https://developer.apple.com/documentation/visionkit/cameraregionview

Gap noted from the official page: `CameraRegionView` is an enterprise visionOS API and needs the Camera Region
entitlement. BifrOSt has no such entitlement, so the desktop model shows the stabilisation logic only. Pixel
post-processing (`PixelBufferProcessingContext`, contrast enhancement) is not mirrored.

## REA evidence

- `ev_13929192384d095b484c912f53cff218c346fab881caa214514fbfe356b30e75`: DataScannerViewController page
  (`tools/parity/rea-page.sh`).
- `ev_3fd2610f39f1d90fc2a7ac2a662e3f216686e21de2bfe067a3bc4901dbb2b869`: CameraRegionView page.

## Matrix rows closed (for central update)

| Framework slug | Feature (exact, from `docs/parity-matrix/data/visionkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| visionkit | Barcode and QR scanning through the camera | partial | springboard/sdk/src/data_scanner.rs (DataScanner over springboard/src/shell/camera/qr.rs) |
| visionkit | Stabilised camera region of interest | partial | springboard/ui/src/camera_region.rs (Stabilizer), re-exported from springboard/sdk/src/data_scanner.rs |

## Files

- `springboard/ui/src/camera_region.rs` (new, tests: 6)
- `springboard/ui/src/lib.rs` (one `pub mod camera_region;` line)
- `springboard/sdk/src/data_scanner.rs` (new, tests: 6)
- `springboard/sdk/src/lib.rs` (one `pub mod data_scanner;` line)
- `docs/parity/w5-visionkit.md` (this file)
