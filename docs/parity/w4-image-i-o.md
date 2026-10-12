# Wave 4: Image I/O (pure Rust)

Two pure std modules in `springboard/core/src` close two Image I/O gap rows. Neither module reads files,
decodes pixels into the shell, or opens a socket: the caller passes the bytes it already has.

## What was built

- `springboard/core/src/gifframes.rs`: GIF 87a and 89a reader. Returns every image with its frame rectangle,
  palette (local or global), delay in milliseconds, disposal method and transparent index, plus the NETSCAPE
  loop count. LZW decoding (3 to 12 bit codes, clear and end codes) and de-interlacing are included.
  `composite()` draws the frames onto a transparent RGBA canvas and applies disposal (restore background,
  restore previous), returning one canvas per frame.
- `springboard/core/src/imgprobe.rs`: header probe for BMP and TIFF (little and big endian). Reads width,
  height (BMP top-down height as a magnitude) and bits per pixel from the first directory. Decodes no pixels.

## Official API this mirrors

- Image I/O (`https://developer.apple.com/documentation/imageio`): the Animations section,
  `CGAnimateImageDataWithBlock`, `CGImageSourceAnimationBlock`, `kCGImageAnimationDelayTime`,
  `kCGImageAnimationLoopCount`, and the GIF Image Properties keys. The BMP and TIFF probe has no direct
  Image I/O counterpart; it covers the "Image Properties" and TIFF Image Properties surface for two more formats.

REA evidence (rendered official landing page, `tools/parity/rea-page.sh`):
- `ev_61ab15b1a6f3b17acd7b0dafe37d787d7d3d346a4ade722d5fb0e74c32079c59` (Image I/O landing page, title
  "Image I/O | Apple Developer Documentation").

## Rows closed

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| Image I/O | Animated GIF and APNG frames | partial | springboard/core/src/gifframes.rs |
| Image I/O | Format coverage (GIF, HEIC, WebP, TIFF, TGA, DNG, RAW) | partial | springboard/core/src/imgprobe.rs |

Still open: APNG frames, GIF and TIFF pixel decode into the SDK, HEIC, TGA, DNG and RAW, and no SDK
(`bifrost_app`) API over either module yet. Neither module is wired into the browser or the Photos app.

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked values: LZW code packings checked bit by bit,
a 2x1 GIF with delay, loop count, disposal and transparency, composited pixels, interlace row order, and
BMP and TIFF (little and big endian) header values. These tests were not run in this wave (no builds).
