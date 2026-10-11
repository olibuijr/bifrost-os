# Theme 33: scanner discovery and functional units

Pure Rust models in `springboard/ui/src` for the device side of image capture: finding cameras and
scanners, the functional units and scan jobs of a scanner, the contents, deletion and control state of a
camera, and timestamp-matched groups from several capture outputs. Each module holds the rules and
state; the transport that talks USB, PTP or the scanner protocol is not in this change, so every row
below closes as `partial`, not `parity`.

Nothing here was compiled or run. This change was written with no builds (the parity run forbids them),
so `cargo test -p bifrost-ui device_browser scan_job camera_device capture_sync` is the first thing to run.

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/device_browser.rs` | `ICDeviceBrowser`, `ICDeviceBrowserDelegate`, `ICDevice`, `ICDeviceType`, `ICDeviceTypeMask`, `ICDeviceTransport`, `preferredDevice`, `browsedDeviceTypeMask`, `start()`, `stop()` | A browser that holds a kind mask (camera, scanner), starts and stops, takes device snapshots and reports `Added` and `Removed` events in a fixed order, and remembers a preferred device across unplug and replug. |
| `springboard/ui/src/scan_job.rs` | `ICScannerDevice`, `ICScannerFunctionalUnitType`, `ICScannerFunctionalUnitState`, `ICScannerTransferMode`, `ICScannerDeviceDelegate`, `requestSelect`, `requestOverviewScan`, `requestScan`, `cancelScan`, `requestOpenSession(withCredentials:password:)`, `defaultUsername`, `transferMode`, `documentUTI`, `downloadsDirectory`, `maxMemoryBandSize`, `IKScannerDeviceView` (display mode, labels, downloads and post-process controls) | The functional units and their resolutions, selection, the session gate for protected scanners (the password is only checked for presence and never stored), one running job at a time, validated settings, and the scan panel config. |
| `springboard/ui/src/camera_device.rs` | `ICCameraDevice`, `ICCameraItem`, `ICCameraFile`, `ICCameraFolder`, `ICDeleteResult`, `ICDeleteError`, `ICDeviceCapability`, `requestDeleteFiles`, `isLocked`, `requestEnableTethering`, `requestTakePicture`, `requestSyncClock`, `timeOffset`, `batteryLevel` | A tree of folders and files with paths and media filters, capability bits, a per-file delete result (locked, not supported, not found, not a file), tethering and capture gates, clock offset and battery level. |
| `springboard/ui/src/capture_sync.rs` | `AVCaptureDataOutputSynchronizer`, `AVCaptureDataOutputSynchronizerDelegate`, `dataOutputs` | Groups samples from several outputs by timestamp. Output 0 is the primary; a group forms when every other output has a sample within the tolerance, the nearest one is chosen, and a primary sample that no partner can match is dropped. |

The four modules are declared in `springboard/ui/src/lib.rs`. Each has a `#[cfg(test)] mod tests` block
with 7 to 8 behaviour tests.

## REA evidence (official pages, rendered in Chromium)

| Official page | REA evidence id |
|---|---|
| imagecapturecore/icdevicebrowser | ev_0a787ee5e5a734c2c0edfb8b8c868cf5a56fa8979e15a3ff33480ea551a1a45d |
| imagecapturecore/icscannerdevice | ev_c8712c2f06811524a49c382e85ddf96ff8ea40637c658f2c0e506f3f4e88a67c |
| imagecapturecore/icscannerfunctionalunittype | ev_b879947694f4f1696c654faa3cf3c7e2018f1fcf249c26551d90cd2bcf0acca1 |
| imagecapturecore/icscannerfunctionalunitstate | ev_02b42773c4f2f69e2917b6858704b852cb16410589c6be97936ff32c75f30c14 |
| imagecapturecore/icscannertransfermode | ev_f9caa04a9cd8e521f13df8f863b76f245fb32ea72e0a5d1097be9b3f374640dc |
| imagecapturecore/icdevice | ev_95dd6c7c2195679813e6e9f283769a9bd1ec1e07468f24dc4b57e2c925c44a31 |
| imagecapturecore/icdevicetype | ev_18cea485a216a4d2fb96f76ba21acf26d681d560221287020fa2ca7f9e7ec264 |
| imagecapturecore/icdevicecapability | ev_d966aeb09a0ac05fc1e0d56873857b1341c0f8b8d0c9c9b4447e604e0cf68f82 |
| imagecapturecore/iccameradevice | ev_9703a327647fd99d25adc10f7f33cd6c6fbbaf1ee607ea9b6641140b4d9e8bfe |
| imagecapturecore/iccameraitem | ev_1169606e3b77fc8f09a1438177d0a0bba3b5e7948d0fc131ef38e9bfcf753cac |
| imagecapturecore/iccamerafile | ev_b3bb78c28026d7b801c54e96c275f7c4df884844c248c2f52dc4301bee6ffaa2 |
| imagecapturecore/iccamerafolder | ev_206b564d23bcea3a9f4359e8ff3b1a569c2c48ca9240d672cb2e868f957667ce |
| avfoundation/avcapturedataoutputsynchronizer | ev_e41140a5c91d72ab2b72e010a315f75931f52346ae84f5919a226cc24ebbc23f |
| quartz/ikscannerdeviceview | ev_fa75d548f10523e46a1c52e4dd91e766d8b9a52be37c2a62185028f30d7c07eb |

Collected but not used by a closed row (Model I/O camera row left as `gap`):
modelio/mdlcamera ev_9144c5618e5f80a8258ec0499b7cf50a2b9250c21d81a9405f8cc1fe707ab700,
modelio/mdlstereoscopiccamera ev_e529da375a2c44eeadcc1d3c08d0eefc62b583cd275b978400dc96c0a95e5947.

## Matrix rows closed

Feature names are as written in `docs/parity-matrix/data/<slug>.json`. Status is `partial` for all rows,
because the transports (USB, PTP, scanner protocol, the capture driver) are not built.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| imagecapturecore | Scanner discovery and functional units | partial | springboard/ui/src/scan_job.rs (units, selection); springboard/ui/src/device_browser.rs (scanner kind) |
| imagecapturecore | Device browser (discovery and hotplug) | partial | springboard/ui/src/device_browser.rs |
| imagecapturecore | Camera device control (PTP tethering, capture, clock, battery) | partial | springboard/ui/src/camera_device.rs (tethering and capture gates, clock offset, battery) |
| imagecapturecore | Camera file browsing and download (contents, files, folders) | partial | springboard/ui/src/camera_device.rs (CameraContents tree, paths, media filters; no download) |
| imagecapturecore | Device-specific constants and enumerations | partial | springboard/ui/src/device_browser.rs (DeviceKind, Transport, DeviceMask); springboard/ui/src/camera_device.rs (Capabilities, Media, EntryKind); springboard/ui/src/scan_job.rs (FunctionalUnitType, TransferMode, DocumentFormat, JobKind) |
| imagecapturecore | Scanning (overview, scan, transfer mode, document type) | partial | springboard/ui/src/scan_job.rs (request_overview, request_scan, cancel_scan, finish_job, configure) |
| imagecapturecore | Camera deletion and locked-device state | partial | springboard/ui/src/camera_device.rs (request_delete with per-file results, is_locked) |
| imagecapturecore | Protected devices (session login, default username) | partial | springboard/ui/src/scan_job.rs (open_session gate, default_username; the scanner checks the password) |
| avfoundation-capture-audio | Additional data capture (depth, metadata, synchronizer) | partial | springboard/ui/src/capture_sync.rs (synchronizer only; depth and metadata objects not built) |
| quartz | Scanner view (IKScannerDeviceView) | partial | springboard/ui/src/scan_job.rs (ScanPanel: display mode, labels, downloads and post-process controls) |

Not closed: model-i-o / "Cameras" (MDLCamera, MDLStereoscopicCamera). Projection and stereo view math
would go in a separate module; it was left out to keep this change to the device side.

## Known gaps left open in this theme

- No transport: no USB enumeration or hotplug source, no PTP session, no scanner driver, no file download.
  The modules are fed by the system layer and do not reach hardware.
- The protected-device password is only checked for presence. The scanner decides whether it is right.
- The scan job does not produce image data or an overview image; it reports start and finish only.
- Model I/O cameras (the one row not closed) remain a gap.
