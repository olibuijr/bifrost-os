# Wire 6: camera devices (device browser, camera picker, Import from Camera)

This round wires the device-side models from theme 33 (`springboard/ui/src/device_browser.rs`,
`springboard/ui/src/camera_device.rs`) into the shell Camera app. Nothing was built or run on the host
(the wiring rules forbid builds); the lead builds once. The first checks are the tests named under
"Verify" below.

## What is now wired

1. **Camera picker with hot-plug** (`springboard/src/shell/camera/mod.rs`, `devices.rs`).
   The app's cameras are the V4L2 capture nodes. The `DeviceBrowser` holds them by a stable id (the card
   name; a second camera with the same name gets `#2`). The switch button (bottom of the control column,
   shown with two or more cameras) moves to the next camera and remembers it as the preferred camera, so
   the same camera opens the next time the app is started.
   Plug and unplug are noticed while the app is open: the `/dev/video*` names are re-read about once a
   second (`HOTPLUG_MS`). A change re-probes the nodes and applies the add and remove events:
   - the streaming camera keeps streaming when it is still attached;
   - when it is unplugged, the preferred camera (or the first one left) opens;
   - a camera that appears while the screen says "Camera not available" opens without Try Again.
   With no camera the screen still says "Camera not available" with Try Again.

2. **Import from Camera** (`springboard/src/shell/camera/import.rs`, `devices.rs`).
   A new **Import** button sits in the top toolbar of the Camera app (4th slot, next to Timer, Grid and
   QR), and is shown in every state except before the app is opened. It opens a full view with:
   - a header: Back, the title, and Import (enabled when at least one picture is picked);
   - a source button that cycles through the attached cards and camera storage (any mounted volume with a
     `DCIM` folder, read from `/run/media/<user>/<label>` and `/media/<user>/<label>`); the first one that
     appears is read, and the view follows hot-plug (a removed card clears the list);
   - a paged list of the JPEG and PNG pictures under `DCIM` (subfolders included, hidden entries skipped,
     each folder sorted by name), with file sizes; a tap picks or un-picks a picture;
   - Previous and Next buttons, and a status line ("N pictures", "Importing ...", "Imported N pictures to
     Photos", "The card was removed").
   Import copies the picked files into the Photos folder (the Camera app's folder, `photo_dirs.first()`),
   keeping each file's modification time. A name that is already taken gets " 2", " 3" before the
   extension, so nothing is overwritten. The copy runs on a thread; the view keeps the frame loop awake
   until it finishes.

   Raw files (CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2, HEIC), movies and audio are listed by the model but not
   shown in the import list, because Photos only reads JPEG and PNG (`imgdec::is_image_name`).

## How a user reaches it

- Cameras: open Camera. With two or more cameras, the round button at the bottom of the control column
  switches to the next one; the choice is kept.
- Import: open Camera, tap **Import** at the top (next to Timer, Grid and QR). Insert a card or connect a
  camera with pictures in its DCIM folder, pick the pictures, tap **Import**. Open Photos to see them.

## How to verify on the test PC (Midget)

Midget is the test machine (`ssh midget`, `bifrost pc midget state`, `bifrost pc midget screen PATH`).
After the round is published and Midget is updated:

1. **Cameras (picker and hot-plug).** Open Camera with the built-in camera. Check the state line:
   `camera_state=live camera_nodes=N camera_current=0`. Plug a USB webcam in while the app is open. Within
   about two seconds the switch button appears (`camera_nodes` increases). Tap the switch button; the
   picture changes source. Unplug the webcam while it streams: the picture moves to the other camera or
   the screen says "Camera not available" when none is left. Plug it back in: it opens without Try Again
   when nothing is streaming.
2. **Import from Camera.** Insert an SD card with a `DCIM/<folder>/` tree of JPEG pictures (or connect a
   camera with mass storage) and make sure the automounter mounts it under `/run/media/<user>/<label>`
   (udisks2 does this on the default desktop; check with `findmnt`). Open Camera, tap **Import**. The
   source button names the card. Tap pictures to pick them (the row turns blue), tap **Import**; the
   status reads "Imported N pictures to Photos". Open Photos and check the pictures are there with their
   capture dates. Import the same pictures again: the copies get " 2" names.
   Remove the card while the view is open: the status reads "The card was removed".
3. **Screenshots.** Take a Midget screenshot of the Import view and of the Camera toolbar, and check them
   against the design rules with the `bifrost-hig` checklist.

Known Midget limits: the Midget lab machine may have no USB camera or card reader. Use the built-in
camera for the picker check, and a card reader (USB) for the import check.

## REA evidence

Official pages were not fetched again in this round: the evidence below is from theme 33
(`docs/parity/theme-33.md`), which collected the ImageCaptureCore pages used here.

| Official page | REA evidence id |
|---|---|
| imagecapturecore/icdevicebrowser | ev_0a787ee5e5a734c2c0edfb8b8c868cf5a56fa8979e15a3ff33480ea551a1a45d |
| imagecapturecore/icdevice | ev_95dd6c7c2195679813e6e9f283769a9bd1ec1e07468f24dc4b57e2c925c44a31 |
| imagecapturecore/icdevicetype | ev_18cea485a216a4d2fb96f76ba21acf26d681d560221287020fa2ca7f9e7ec264 |
| imagecapturecore/iccameradevice | ev_9703a327647fd99d25adc10f7f33cd6c6fbbaf1ee607ea9b6641140b4d9e8bfe |
| imagecapturecore/iccameraitem | ev_1169606e3b77fc8f09a1438177d0a0bba3b5e7948d0fc131ef38e9bfcf753cac |
| imagecapturecore/iccamerafile | ev_b3bb78c28026d7b801c54e96c275f7c4df884844c248c2f52dc4301bee6ffaa2 |
| imagecapturecore/iccamerafolder | ev_206b564d23bcea3a9f4359e8ff3b1a569c2c48ca9240d672cb2e868f957667ce |

Behaviour taken from these pages: browser events are reported as added and removed devices, the device keeps
its identity across unplug and replug (the model's stable id), and the camera's contents are a tree of
folders and files whose paths the import joins onto the device root.

## Tests

- `springboard/src/shell/camera/devices.rs`: id numbering for repeated card names; media kind by extension;
  volume id to path round trip; the DCIM tree (hidden entries skipped, sorted, paths start at `DCIM`);
  numbered free names; import copies pictures only and never overwrites.
- `springboard/src/shell/camera/import.rs`: page split (`page_count`, `page_range`); size text; the first
  volume is read and removal moves to the next; picking rows builds a copy request under the mount point
  and the status after the copy.
- `springboard/src/shell/camera/mod.rs`: Import opens and closes without touching a device; with no camera
  the screen says "No camera found" and a camera-less app rechecks every second.

Verify: `cargo test --offline -p springboard camera::` (the shell crate's camera tests; run the focused
set first, then the full acceptance once per batch with `bifrost pc acceptance --quick`).

## Remaining gaps

- No PTP or gphoto transport: a camera that exposes only PTP over USB (no V4L2 node, no mass storage) is
  not seen. Cameras appear only as V4L2 capture nodes, and cards only through a mount with DCIM.
- Import reads from a mounted DCIM folder. It does not talk to a camera over PTP to list or copy its
  files (`ICCameraDevice.requestDownloadFile` and PTP tethering are not built).
- Raw, HEIC and movie files are listed by the model but not imported (Photos reads JPEG and PNG only).
- Deleting files on the card and the camera clock are not wired.
- The model's device kinds have no storage kind, so cards are recorded as `DeviceKind::Camera` with
  transport `Other`. Splitting storage out needs a change to `device_browser.rs` (a shared file).
- The Import button and the picker are drawn with the Camera app's own controls, not the kit widgets. The
  Import view uses the shell's text cache and the shared colour tokens.
- Glue in `mod.rs` (hot-plug events, the preferred camera on switch) has only the tests listed above; the
  plug and unplug path is verified on Midget, not by a host test.

## Matrix rows

Feature text is exactly as written in `docs/parity-matrix/data/<slug>.json`. Status in the matrix is not
changed by this note: a row becomes `parity` only once it works on Midget (owner rule).

| Slug | Feature | Wired now | BifrOSt path |
|---|---|---|---|
| imagecapturecore | Device browser (discovery and hotplug) | camera picker, hot-plug of nodes, volume browser for the Import view | springboard/src/shell/camera/mod.rs (check_devices, refresh_cameras, open_import); springboard/src/shell/camera/devices.rs (camera_devices, volume_devices); springboard/ui/src/device_browser.rs |
| imagecapturecore | Camera file browsing and download (contents, files, folders) | Import from Camera: list DCIM tree of a card or camera storage and copy picked pictures to Photos | springboard/src/shell/camera/import.rs; springboard/src/shell/camera/devices.rs (contents_of, import_files); springboard/ui/src/camera_device.rs |
| avfoundation-capture-audio | Capture setup (devices, sessions, inputs) | camera discovery for the Camera app (preferred camera, switch, hot-plug) | springboard/src/shell/camera/mod.rs; springboard/src/shell/camera/devices.rs (camera_ids) |
