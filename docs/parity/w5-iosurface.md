# Parity wave 5: IOSurface buffer models

Framework: IOSurface (`docs/parity-matrix/data/iosurface.json`, status `os`). Official page:
https://developer.apple.com/documentation/iosurface

## What was built

Three pure Rust models in `springboard/ui/src/` (std only, no unsafe, no hardware access). They are not yet wired
into the shell or the SDK, and they have not been built or run on the VM or Midget.

- `surface_lock.rs`: `SurfaceLock`, the CPU lock state of a shared buffer. Any number of read locks may be held
  together, a write lock is exclusive, and releasing a write lock advances a seed. Mirrors the read-only option of
  `IOSurfaceLockOptions` and the `lock(options:seed:)` / `unlock(options:seed:)` pair.
- `surface_layout.rs`: `PixelFormat` (ARGB8888, XRGB8888, RGB565, NV12, I420, named by four-character code) and
  `SurfaceLayout`, which gives each plane's element width and height, bytes per element, row stride padded to an
  alignment, byte offset and size, plus the total size. Mirrors the per-plane accessors (plane count, width, height,
  bytes per row of plane) and the pixel format and bytes-per-row properties.
- `surface_colour.rs`: `ColorSpace` (sRGB, linear sRGB, Display P3, grey) and `SurfaceColor`, which tags a buffer with
  a colour space and an optional ICC profile. The profile is checked for its 128-byte header, its `acsp` signature and
  its data colour space against the tagged space. The tag is stored in the existing attachment dictionary
  (`surface_attach.rs`) under the key `colour-space`.

Each module has a `#[cfg(test)] mod tests` with hand-worked values. The ones for the layout module cover stride
padding (400 bytes padded to 448 at alignment 64), NV12 at 1920 by 1080 (3110400 bytes), I420 at 10 by 4 (60 bytes)
and chroma padding at 100 by 50.

## Official evidence

- Landing page (REA): `ev_cdea2b2b338ad07f687a22c11212c56fa85ee836a79bc731745341e3067011a9`
- Official symbol pages read with `docc.py`: `IOSurface` (instance properties `pixelFormat`, `bytesPerRow`,
  `planeCount`, `isInUse`, `localUseCount`; methods `lock(options:seed:)`, `unlock(options:seed:)`,
  `baseAddressOfPlane(at:)`, `setAttachment(_:forKey:)`), `IOSurfaceLockOptions` (`readOnly`, `avoidSync`).
- The colour-space key names (`kIOSurfaceColorSpace`, `kIOSurfaceICCProfile`) come from the matrix row and were not
  re-read from the official pages in this wave.

## Matrix rows closed

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| IOSurface | CPU lock and unlock of buffer memory | partial | springboard/ui/src/surface_lock.rs |
| IOSurface | Pixel format, stride and plane layout | partial | springboard/ui/src/surface_layout.rs |
| IOSurface | Colour space and ICC profile on buffers | partial | springboard/ui/src/surface_colour.rs |

## Still open

- No app-facing SDK type wraps these models; the SDK and shell wiring is the next step.
- The mapping of the seed and of the reader and writer counts to real mapped memory is not done.
- Multi-plane video buffers stays `os` until the layout model is exposed to apps.
