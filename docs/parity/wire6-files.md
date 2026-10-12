# Wire 6: Files app, image and launch parity (wire6/files)

Scope: `springboard/apps/files/` (Files 1.20.0 to 1.21.0). Three pieces of the image, Get Info and launch models were wired
into the app; the rest of the focus list is recorded as gaps below. Nothing was built or run in this round (the lead builds
once). Every expected value in the tests was worked out by hand from the code.

## What is now wired

1. **Picture thumbnails through the image source.** The icon view (and the gallery and list thumbnails that share
   `blit_thumb`) now decodes pictures through the SDK `image_source::ImageSource`: the header is read first, so a picture
   wider or taller than the decode limit (8192 px) is refused before any pixel is allocated, and the decode is scaled to the
   box, never enlarged. Code: `springboard/apps/files/src/views/options.rs` (`picture_thumb`, `blit_thumb`).
   The decode is still full size before the scale, so this is a header check plus the shared fit, not a reduced-resolution
   decode.
2. **EXIF section in Get Info.** For a `.jpg` or `.jpeg` file, Get Info reads the first 128 KB and shows the rows the
   Photos app's EXIF reader gives (`springboard::shell::photos::exif`, pub from the springboard crate that Files already
   depends on): Date, Camera, Exposure, Dimensions and Location (GPS, when the photo has it). The reader's "Size" row is
   labelled "Dimensions" because Get Info already has a Size row for the file. Code: `springboard/apps/files/src/info.rs`
   (`exif_rows`, `exif_rows_from`, called from `get_info`).
3. **Open With… context menu item and panel.** On a single file (not a folder), the context menu has "Open With…". It sends
   the launch service's `open_with` call through the system-service queue (`Effect::OpenWithApps`, `Waiting::OpenWith`
   in `springboard/apps/files/src/system.rs`). The reply fills a panel (`springboard/apps/files/src/openwith.rs`) under the
   Get Info position: "Looking up apps…" while it waits, then each capable app, the default marked "(Default)", or the one
   line that explains the answer (no app, or the service's error). Escape or a click closes it. Rows are read-only; the
   footer points to Settings > General > Default Apps.
4. **Recents** was already the first sidebar row (`springboard/apps/files/src/sidebar.rs`, `fs::RECENTS`). Nothing changed.
5. **Colour.** The main content area already fills with `CONTENT_BG` (`window.rs`). The View Options card fills with
   `DETAIL_BG`; it is a floating card, not the content area, and it was left as it is.

Pure tests (host, not run): `openwith.rs` (`tests`: default first and marked, pending text, empty answer and error each one
line, unmarked candidate), `info.rs` (`image_tests`: a JPEG with only a SOF0 header shows `("Dimensions", "20 x 10")`, a PNG
or a missing file shows no EXIF rows), `views/options.rs` (`thumb_tests`: a 4000 x 3000 header fits a 200 x 200 box at
200 x 150; an 8193 px header is `TooLarge` and gives no thumbnail; non-image bytes give none).

## How a user reaches it

- Thumbnails: open a folder with PNG or JPEG pictures in icon view. Each picture shows its own
  picture, scaled into the icon box, as before; this round changes how the size is decided, not what is shown.
- Get Info: select a JPEG and press Command-I (or choose Get Info from the context menu). The EXIF rows follow Kind, Size,
  Where and the dates.
- Open With…: right-click a file (single selection) and choose "Open With…". Escape or a click closes the panel.

## How to verify on Midget

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Thumbnails: copy a large JPEG and a PNG into `~/Pictures`. Icon view shows both as pictures in their icon boxes.
   Expected: no crash, no blank box for either.
3. EXIF: copy a phone JPEG (one with a date and GPS) to `~/Pictures`, select it, Get Info. Expected rows include `Date`,
   `Camera`, `Exposure`, `Dimensions` and, with GPS, `Location` (`52.5200 N, 13.4050 E` style). A PNG shows none of these.
4. Open With: select a `.txt` file, right-click, Open With…. Expected: the panel shows "Looking up apps…" and then the
   service's answer (with no app declaring the type, "No app opens this file."). Escape closes it.
5. Recents: the sidebar's first row is Recents and opens the recent-items list.

## REA evidence

- `ev_00f8a49ae88d37ce440db5a8666fecc0c80bf11895946c53d46bd5d4f8b215ac`: rendered official page
  `https://developer.apple.com/documentation/imageio/cgimagesource` (Image I/O: creating an image source from data, the
  thumbnail creation options `k CGImageSourceCreateThumbnailFromImageAlways` and `...IfAbsent`, and the properties
  functions). `python3 -I tools/parity/docc.py imageio/cgimagesource` lists the same symbols, including
  `CGImageSourceCreateThumbnailAtIndex`.
- `ev_057bd6e8bbc652004de7bd8176d37e6f0a4c90ee28490ec073141ee948c3ec9f9`: rendered official page
  `https://developer.apple.com/documentation/coreservices/launch_services` (Locating an App: `LSCopyApplicationURLsForURL`,
  "Locates all known apps suitable for opening an item"; the quarantine properties keys `LSQuarantine*`). The
  `docc.py coreservices/launch_services` output matches.

## Remaining gaps

- **Opening a file in a chosen app from Open With.** Not wired. The shell's workspace `open_with` (`bifrost_app::workspace`)
  takes document grants the app holds, and Files holds none for the files it browses; Files opens with the default app
  through `bifrost-apps --open`. A chosen-app open needs a shell call that takes a path or an app-owned grant.
- **Open With as a submenu.** Context menus are flat (`ContextItem` has no children), so the list is a panel, not a submenu.
- **Quarantine.** `springboard/core/src/quarantine.rs` (`Quarantine`) is not wired. Reading the quarantine attribute of a
  download needs extended-attribute access, which std does not provide (the same gap as Get Info's extended attributes).
  Downloads are not tagged, so there is nothing to show yet.
- **Compression formats.** Compress and Extract still use ZIP only; no other archive container is built.
- **Thumbnails are not reduced-resolution decodes.** The source is decoded in full and then scaled; a decoder that
  downsamples while reading the file is not in the renderer.
- **Get Info location is read-only and has no SDK API.** The GPS value is shown as text; no app can read photo metadata
  through the SDK yet (the Photos reader is shell-only, and moving it into `springboard/ui/src` is the next step).
- **Image view and browser for apps** (Quartz row) is not touched.
- Not verified: no build or test run (the lead builds once). The compliance and snapshot tests in `springboard/apps/files/tests/`
  do not cover the new rows or the Open With panel; the lead should bless the snapshot only if one shows the Get Info panel.
- `springboard/Cargo.lock` has the Files version line from the bump; the lead's build updates it (not edited here).

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| image-i-o | Downsampled and thumbnail decode | springboard/apps/files/src/views/options.rs (picture_thumb, blit_thumb) |
| image-i-o | Image source: decode from data or file | springboard/apps/files/src/views/options.rs (picture_thumb) |
| image-i-o | EXIF metadata read (camera, date, exposure, size) | springboard/apps/files/src/info.rs (exif_rows, exif_rows_from); springboard/src/shell/photos/exif.rs (parse, rows) |
| core-services | Listing every app that can open an item | springboard/apps/files/src/openwith.rs (OpenWith::lines, paint_open_with); springboard/apps/files/src/system.rs (Waiting::OpenWith) |

Rows are not promoted to `parity` here. The Downsampled and EXIF rows need the Midget checks above; "Listing every app that
can open an item" stays partial until a chosen-app open exists. The Quarantine row (`core-services`, "Quarantine properties")
is not changed. `docs/parity-matrix/` is shared and was not edited.
