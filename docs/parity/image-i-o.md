# Image I/O (parity)

Official reference: https://developer.apple.com/documentation/imageio

This page tracks the Image I/O rows of `docs/parity-matrix/data/image-i-o.json`. The codecs live in
`springboard/render/src/cg/imageio` (PNG, JPEG, GIF, BMP, ICO and TIFF read and write; WebP read only). This round
exposes them to SDK apps and adds the metadata and animation APIs the matrix listed as system-only.

## Status

Built on branch `parity/team-n-image-i-o` (from `arm`). Nothing here has been built or run yet: the code was written
without cargo, and no test has run on the host or on Midget. Every row stays `partial` until it works on Midget.

## Modules (SDK, `springboard/sdk/src`)

| Module | Mirrors | What apps get |
|---|---|---|
| `graphics_imageio` | `CGImageSourceCreateWith*`, `CGImageDestination` (basic) | `detect`, `pixel_size`, `decode`, `encode`, `encode_jpeg`, `can_write` |
| `image_source` | `CGImageSource` (header first, thumbnails) | `ImageSource::new`, `size`, `decode`, `thumbnail`, `representation` |
| `image_metadata` | EXIF, GPS and orientation dictionaries (`kCGImagePropertyExif*`, `kCGImagePropertyGPS*`, `kCGImagePropertyOrientation`) | `read` (`PictureInfo`: size, orientation, dpi, camera, capture date, exposure, f-number, ISO, focal length, location), `decode_upright`, `dms_to_decimal`, `decimal_to_dms` |
| `image_animation` | `CGAnimateImageDataWithBlock`, `kCGImageAnimationDelayTime`, `kCGImageAnimationLoopCount` | `read` (`Animation` with frames, delays, loop count, `duration_ms`) |
| `picture_writer` | `CGImageDestinationAddImage` with properties, `CGImageDestinationFinalize` | `PictureWriter` builder: `quality`, `orientation`, `date_taken`, `location`, `loop_count`, `add_frame`, `finish` |

## Behaviour notes

- Coordinates are decimal degrees, north and east positive. EXIF stores degrees, minutes and seconds with a
  hemisphere letter; `dms_to_decimal` rejects latitudes above 90 and longitudes above 180.
- `picture_writer` writes the orientation, capture date and location into the formats that carry EXIF (JPEG, PNG,
  TIFF). GIF, BMP and ICO keep only pixels and frame timing. An orientation outside 1..=8 and a date that is not
  `YYYY:MM:DD HH:MM:SS` are ignored.
- `image_animation::read` returns one frame with delay 0 for a still picture.
- Pixels passed to the writers are straight RGBA, four bytes per pixel, rows top to bottom.

## Still open

- XMP and IPTC metadata (the EXIF reader skips XMP packets and maker notes as opaque).
- HEIC, TGA and RAW: not recognised by the codecs.
- The Photos and camera code still use their own encoders (`springboard/src/pngenc.rs` writes uncompressed PNG,
  `springboard/src/shell/camera/jpeg.rs` writes the camera JPEG). They are not moved onto `picture_writer`.
- ICC and HDR gain maps: images are treated as sRGB.
- Tiled or streaming providers for very large pictures (`springboard/render/src/image_tiles.rs` is a grid model only).
- APNG frame access is not verified.

## Tests (hand-worked values, not yet run)

- `image_metadata`: DMS `[64, 8, 30.5]` gives 64 + 8/60 + 30.5/3600; S and W are negative; `[21, 30, 0]` south-west
  gives -21.5; out-of-range and non-finite parts are refused; a PNG has size and no camera data; a 2 x 1 PNG written
  with orientation 6 decodes upright as 1 x 2.
- `image_animation`: a GIF with delays 100 and 200 and loop count 3 reads back the same; a PNG is one frame with
  delay 0.
- `picture_writer`: the EXIF date shape check; a JPEG round trip keeps orientation 6, the date and the location
  (64.25, -21.5); an orientation of 9 and a bad date are dropped; a two-frame GIF keeps delays 50 and 150 and loop
  count 2; a wrong pixel count is refused; WebP and an empty writer give nothing.

## Verification

- Lead to run: `cargo test -p bifrost-app image_metadata image_animation picture_writer` on the host, then the
  SDK coverage check (`tools/check-sdk-coverage.py`) and `python3 tools/check-names.py`.
- Midget: not yet tested. Each row stays `partial` until a Midget check passes.
