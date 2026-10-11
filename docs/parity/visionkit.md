# VisionKit parity

Framework: VisionKit (https://developer.apple.com/documentation/visionkit). Status: partial.
Matrix data: `docs/parity-matrix/data/visionkit.json`.

## What was built

Three pure-Rust models in `springboard/ui/src` (bifrost-ui, std only, no unsafe, no camera, no network). None is
wired to a camera feed, a text recognizer or an SDK API yet.

| Module | Mirrors | What it does |
|---|---|---|
| `scan.rs` (`ScanSession`, `Item`, `ItemKind`, `Quad`, `Changes`) | `DataScannerViewController` text items, `RecognizedItem`, the delegate's add/update/remove callbacks | Keeps the visible items by stable id. Each frame's observations give the added, updated and removed ids. A repeated identical frame is silent. Hit testing finds the items under a point, topmost first. |
| `analysis.rs` (`Overlay`, `TextBlock`, `Selection`, `Position`) | `ImageAnalysisOverlayView` and `ImageAnalysisInteraction` text selection, data detectors in image text | Word selection by character class, ranges across blocks in reading order (either end first), select-all, copied text with a newline between blocks, and data detection inside the selection only, with matches mapped back to block offsets. |
| `docscan.rs` (`DocumentScan`, `Page`, `order_corners`, `page_size`, `homography`, `apply`) | `VNDocumentCameraViewController` and `VNDocumentCameraScan` page results | Orders page corners, computes the flat page size from the longer opposite edges, solves the 3x3 homography (Gauss-Jordan with partial pivoting), maps output pixels back to camera points, and keeps the scanned pages in capture order. |

The modules are registered in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests` with real behaviour
tests (five or more per module).

## Apple API shape

Checked with `python3 -I tools/parity/docc.py visionkit` (official documentation, collection page). The models follow
these names: `DataScannerViewController`, `RecognizedItem`, `DataScannerViewControllerDelegate`,
`ImageAnalysisOverlayView`, `ImageAnalysisInteraction`, `ImageAnalysisInteractionDelegate`,
`VNDocumentCameraViewController`, `VNDocumentCameraScan`.

## REA evidence

- `ev_d89b4abf3a05a97aa7678cfb36629c289e35d7ffc9b94746adbc748b48741fe4`: rendered page
  https://developer.apple.com/documentation/visionkit (captured this round with `rea-page.sh`). It describes the
  Image Analysis interface (text selection, data detectors, overlay on macOS), the document camera page flow, and
  subject identification in iOS 17 and macOS 14 and later.
- `ev_a51ac1d6ab8726e08a68c2fed8d36892880a7b75ade4e28d7e31023922e655cf`: the evidence the matrix row already cited.

## Remaining gaps

- No text recognition (OCR) for camera frames or images. The models take recognised text and corners as input.
- No document scanning UI in the Camera app (page edge detection, page-by-page capture) and no SDK API.
- No subject lifting or background removal, and no Visual Look Up.
- No SDK view or delegate for the image-analysis overlay, and no stabilised camera region view (`CameraRegionView`).
- Barcode scanning remains QR model 2 only, inside the system Camera app, with no SDK API.
- Data detection has no transit or tracking-number kind.

## Verification

Not built or run in this round, by instruction: the code is to be compiled and tested by the next stage.
