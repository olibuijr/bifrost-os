# Parity wave 4: Core Video buffers

Branch `parity/w4-core-video`, built on `parity/integration` (waves 1 to 3). Pure Rust, std only, no builds run in
this wave. The matrix (`docs/parity-matrix/data/core-video.json`) is not edited; the lead imports the rows below.

## What was built

Three new pure models in the render crate (`springboard/render/src/`), one `pub mod` line each in `lib.rs`.

- **`pixel_format.rs`** (`bifrost_render::pixel_format`). A `FourCc` code (DRM fourcc byte order, first character in
  the low byte; `parse`, `bytes`, `Display`), a `Plane` (bytes per sample, horizontal and vertical subsampling), and a
  `PixelFormat` with a name and its planes. A static `TABLE` covers ARGB8888, XRGB8888, ABGR8888, XBGR8888, RGB565,
  NV12 and NV16. `find` looks a code up. `plane_size` gives the row bytes and row count of a plane for odd sizes
  (chroma rounds up); `buffer_len` gives the total with checked arithmetic, so an overflowing size returns None.
- **`buffer_pool.rs`** (`bifrost_render::buffer_pool`). A generic `Pool<T>` with a `Config` (minimum count, maximum
  age in caller ticks, allocation threshold). `acquire` reuses the newest free buffer, otherwise makes one under the
  threshold, else returns `PoolError::WouldExceedThreshold`. `release` returns a buffer at a tick. `prewarm` creates
  up to the minimum. `age` drops free buffers idle for more than the maximum age, oldest first, never below the
  minimum. `flush` drops every free buffer and keeps buffers in use. The pool keeps no clock: the caller passes ticks.
- **`buffer_attach.rs`** (`bifrost_render::buffer_attach`). Typed colour attachments on a buffer: `Primaries`
  (BT.709, SMPTE 170M, BT.2020, Display P3), `Transfer` (sRGB, BT.709, linear, PQ, HLG), `YcbcrMatrix` (BT.601, BT.709,
  BT.2020 with `luma_weights`), and a clean aperture `Rect`. `color_space` maps BT.709 or Display P3 primaries with the
  sRGB transfer to the existing `colorspace::Space`, and returns None for anything else. `clean_rect` clips the
  aperture to the buffer size and returns an empty rectangle when the aperture starts outside it. Nothing is inferred
  from pixels.

Every module has a `#[cfg(test)] mod tests` with hand-worked expected values (6 tests in pixel_format, 8 in buffer_pool and 6 in buffer_attach).

No SDK wrapper exists yet. Apps still get no pixel buffer type, pool or attachment API, so the matrix rows below are
`partial`, not `parity`. `springboard/sdk/src/` is the place for a facade over these models in a later wave.

## Apple API names mirrored

Official pages (DocC and the rendered page, REA evidence in the table below):

- `CVPixelBufferPool` (the pool, its minimum count, maximum age and allocation threshold, and flush):
  https://developer.apple.com/documentation/corevideo/cvpixelbufferpool
- `CVPixelFormatDescription` (the name, plane configuration, component range and pixel format type): https://developer.apple.com/documentation/corevideo/cvpixelformatdescription
- Image buffer attachments (`CVBufferSetAttachment`, `CVBufferCopyAttachments`, `CVImageBufferGetCleanRect`):
  the attachments page returned "page not found" on the rendered site. The landing page
  https://developer.apple.com/documentation/corevideo lists `CVImageBuffer` as the owner of these functions, so the
  mapping rests on the landing page and the DocC symbol names, not on a separate attachments reference.

Not mirrored: the CoreVideo type and error codes, the Metal and OpenGL texture caches, the display link, pixel buffer
base-address locking and the IOSurface bridge.

## REA evidence

| Page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/corevideo (framework landing) | ev_aff220b0789a9d73d85e7de2016b9a2167b68510ebed035b91a27337c03a7de9 |
| https://developer.apple.com/documentation/corevideo/cvpixelbufferpool | ev_ef42e173e94e2599d3f74700d05adb1ed73e3caa4c4dd2fb914105cfb70075d2 |
| https://developer.apple.com/documentation/corevideo/cvpixelformatdescription | ev_bd248476d3f06493809ec7bbbc62516d24db02b0fbcf850e396d15fd0e5d40e7 |
| https://developer.apple.com/documentation/corevideo/cvbufferattachments (404 page) | ev_e5ea6ac127f6f24d971ea208de85278c3d41334f713d775e40fd292752625767 |

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| core-video | Pixel format description and fourcc types | partial | springboard/render/src/pixel_format.rs |
| core-video | Pixel buffer pool | partial | springboard/render/src/buffer_pool.rs |
| core-video | Image buffer attachments and colour metadata | partial | springboard/render/src/buffer_attach.rs |

Not closed here: Pixel buffers (CPU-side image buffers). The model covers only format layout, and the lock and base
address surface is still missing. Display timing, texture caches and dma-buf surfaces stay `os`.

## Tests

Run with the usual `cargo test -p` once a build is allowed. This wave ran no build.
