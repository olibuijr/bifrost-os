# Wave 5: Core Video (app-facing pixel buffers and display link)

Branch `parity/w5-core-video`, on top of `parity/integration` (waves 1 to 4). Pure Rust, std only, no unsafe, no
hardware access. Nothing is wired into the shell or into a running app yet, and the code has not been built or
tested on the VM or Midget (the wave's rule is no builds in this round).

## What was built

**`springboard/sdk/src/pixel_buffer.rs`** (app-facing SDK, `bifrost_app::pixel_buffer`)

- `PixelBuffer`: a CPU-side buffer in one of the formats of `bifrost_render::pixel_format` (ARGB8888, XRGB8888,
  ABGR8888, XBGR8888, RGB565, NV12, NV16). Planes are tight-strided and odd chroma sizes round up. Accessors:
  `code`, `format_name`, `width`, `height`, `is_planar`, `plane_count`, `bytes_per_row(plane)`,
  `plane_width(plane)`, `plane_height(plane)`, `data_size`.
- Locking: `lock(read_only)` and `unlock(read_only)` count nested locks. `base_address(plane)` returns bytes only
  while a lock is held. `base_address_mut(plane)` needs a read-write lock. `unlock` without a matching lock returns
  `PixelBufferError::NotLocked`.
- `PixelBufferPool`: a pool of buffers of one format and size, built on `bifrost_render::buffer_pool::Pool`.
  `prewarm`, `acquire`, `release`, `age`, `flush`, `live`, `free_count`. Limits come from `PoolConfig` (the render
  crate's `Config`, re-exported).

**`springboard/sdk/src/display_link.rs`** (app-facing SDK, `bifrost_app::display_link`)

- `DisplayLink`: a display link at an integer refresh rate. `new(hz)`, `refresh_period()` in nanoseconds,
  `start(host_ns)`, `stop()`, `is_running()`, `tick(host_ns) -> Option<TimeStamp>`, `missed_refreshes()`.
- `tick` reports one output time per refresh, once per refresh. It is driven by the app's own ticks from
  `App::next_wake`. Refreshes that passed with no tick are counted as missed.
- `TimeStamp`: the fields of the Core Video time stamp (`frame_number`, `host_time`, `video_time`,
  `video_time_scale` = `VIDEO_TIME_SCALE` (1e9), `rate_scalar` = 1.0, `video_refresh_period`).

## Official API names mirrored

Core Video (https://developer.apple.com/documentation/corevideo):

- `CVPixelBuffer` (https://developer.apple.com/documentation/corevideo/cvpixelbuffer-q2e): `CVPixelBufferCreate`,
  `CVPixelBufferLockBaseAddress`, `CVPixelBufferUnlockBaseAddress`, `CVPixelBufferGetBaseAddress`,
  `CVPixelBufferGetBaseAddressOfPlane`, `CVPixelBufferGetBytesPerRow`, `CVPixelBufferGetBytesPerRowOfPlane`,
  `CVPixelBufferGetWidth`, `CVPixelBufferGetHeight`, `CVPixelBufferGetWidthOfPlane`,
  `CVPixelBufferGetHeightOfPlane`, `CVPixelBufferGetPlaneCount`, `CVPixelBufferIsPlanar`,
  `CVPixelBufferGetDataSize`, `CVPixelBufferGetPixelFormatType`.
- `CVPixelBufferPool` (landing page entry "CVPixel Buffer Pool"): the pool model over the render crate's buffer pool.
- `CVDisplayLink` (https://developer.apple.com/documentation/corevideo/cvdisplaylink): the refresh and time-stamp
  side of the class: `CVDisplayLinkGetActualOutputVideoRefreshPeriod`, `CVTimeStamp`, start and stop. The per-display
  output callback and `CVDisplayLinkCreateWithActiveCGDisplays` are not mirrored (see gaps).

## REA evidence

- Core Video landing page: `ev_61a6ce01a8bb7793610613bd29bb87ee07227942da1696cdabdb66f434761006`
- CVPixelBuffer: `ev_9c13ae502c274be5dbb8c5e584505dc9691a293d2081b8c4db2e0cbbcc17d220`
- CVDisplayLink: `ev_3b2a20b8186935593c8c78c858813a41957386af105841341b68c83385b788ec`

## Matrix rows closed

These rows are for the lead to update in `docs/parity-matrix/`. Not edited here.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-video | Pixel buffers (CPU-side image buffers) | partial | springboard/sdk/src/pixel_buffer.rs |
| core-video | Pixel buffer pool | partial | springboard/sdk/src/pixel_buffer.rs (PixelBufferPool) |
| core-video | Display timing and vsync (CVDisplayLink, CVTime, CVTimeStamp) | partial | springboard/sdk/src/display_link.rs |

## Gaps that remain

- Pixel buffers are not shared with the presenter or with the GPU. The GPU path still allocates its own GBM buffers,
  and an app cannot render into a presenter drawable through this API.
- The display link has no per-display output callback, no CVDisplayLinkCreateWithActiveCGDisplays equivalent, and no
  link to the KMS vsync. The timing is driven by the app's own ticks.
- Image buffer attachments (colour primaries, transfer function, YCbCr matrix, clean aperture) are not carried on the
  pixel buffer. The render crate's buffer attach model covers crop and stride only.
- No texture cache: an app cannot import a pixel buffer as a GPU texture.
- Core Video result codes (`CVReturn`) are not mirrored: errors are `PixelBufferError` values.

## Tests

`cargo test` was not run (no builds this round). Each new module has a `#[cfg(test)] mod tests` with hand-worked
expected values:

- `pixel_buffer.rs`: NV12 odd-size plane rounding (5 x 3 gives 5, 6 bytes per row and 27 total), unknown format and
  empty size refusal, bytes hidden until locked, a write under a read-write lock read back, nested locks and
  unlock without a lock, and the pool's reuse and threshold (2 buffers at most, flush leaves 1 live).
- `display_link.rs`: refresh period from the integer rate (100 Hz gives 10_000_000 ns, 60 Hz gives 16_666_666 ns),
  a stopped link reports nothing, one report per refresh, skipped refreshes counted as missed (a tick at 35 ms on a
  100 Hz link is refresh 3 with 2 missed), start offset and restart resetting the count, and a tick before the start
  counted as refresh 0.

## Files changed

- `springboard/sdk/src/pixel_buffer.rs` (new)
- `springboard/sdk/src/display_link.rs` (new)
- `springboard/sdk/src/lib.rs` (two `pub mod` lines)
- `docs/parity/w5-core-video.md` (this page)
