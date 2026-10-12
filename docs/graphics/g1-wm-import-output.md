# G1 wave: window server importer and output targets (`wm-import-output`)

Part of the graphics core rebuild (`docs/graphics-architecture.md`, "Window server"). Owner files:
`springboard/wm/src/render_server/import.rs` and `output.rs`.

## What is implemented

### `import.rs`: client buffers to textures

- `Importer::import_dmabuf(key, &DmaBuf)`: imports through `Device::import_dmabuf` once per key and caches the
  `SurfaceTexture` (`zero_copy: true`). A repeated call with the same key returns the cached texture and does not
  look at the descriptor again. `forget(key)` drops it.
- `Importer::upload_shm(surface_key, w, h, stride, bytes, damage)`: BGRA8/BGRX8 rows (4 bytes per texel). The
  per-surface texture is created as `Bgra8Unorm` with `SAMPLED | COPY_DST`, and recreated (fully uploaded) when the
  size changes or the surface is new. Otherwise only the damage rectangles are uploaded, each through
  `Texture::replace_region` with the source stride. Damage is `(x, y, width, height)` in buffer pixels and is clipped
  to the buffer; empty or outside rectangles are dropped. Buffers whose stride is shorter than a row, or whose bytes
  are shorter than `shm_span`, are refused with `Error::Invalid`.
- `dmabuf_to_gpu(&Dmabuf) -> bifrost_gpu::Result<DmaBuf>`: width and height from `Buffer`, fourcc from
  `format().code`, modifier as `u64` from `format().modifier`, each plane's fd duplicated
  (`BorrowedFd::try_clone_to_owned`) with its offset and stride. A failed duplicate becomes `Error::DmaBuf`.
- Pure helpers, host-tested: `shm_span`, `clamp_damage`.

### `output.rs`: scanout ring and direct-scanout check

- `OutputTargets` keeps the public fields of the skeleton. A private `slots` vector tracks each buffer as `Free`,
  `Acquired` (rendering, or submitted and waiting for its flip) or `OnScreen`.
- `acquire()` returns the first free buffer scanning from `next` (wrapping) and advances `next`. `None` when every
  buffer is acquired or on screen.
- `flipped(i)` moves an acquired buffer on screen and frees the previous on-screen buffer. A flip for a buffer that
  is not acquired (duplicate or stale event) changes nothing.
- `abandon(i)` (added) frees an acquired buffer whose frame was never shown.
- `free_count()` (added) reports how many buffers are free, for the frame stats.
- `from_textures(w, h, targets)` (added) builds the ring over existing textures, for the nested backend.
- `from_gbm(device, w, h, &[GbmBuffer])` (feature `drm`): exactly `RING` (3) buffers of the output size. Each is
  exported with `AsDmabuf::export`, converted with `dmabuf_to_gpu`, and imported as a render target with
  `Device::import_dmabuf`. The caller keeps the GBM buffers alive for the KMS framebuffers.
- `direct_scanout_eligible(&ScanoutCandidate, out_w, out_h, &[DmaBufFormat]) -> bool`: the candidate must be the
  output size, its exact (fourcc, modifier) pair must be in the plane's list, and it must be opaque (opaque region,
  X format) or have no surface below it.

## Mapping to the contract

| Contract item | Where |
|---|---|
| `Importer` dma-buf import, cached per buffer | `import.rs`, `Importer::import_dmabuf`, `forget` |
| shm upload, per-surface texture, damaged rows, recreate on size change | `import.rs`, `Importer::upload_shm` |
| Smithay to `bifrost_gpu::DmaBuf` helper | `import.rs`, `dmabuf_to_gpu` |
| Ring of 3 with acquire and flipped bookkeeping | `output.rs`, `OutputTargets::{acquire, flipped}` and `ring_*` |
| GBM buffers exported as dma-bufs and imported as render targets (feature `drm`) | `output.rs`, `OutputTargets::from_gbm` |
| Direct-scanout eligibility as a pure function | `output.rs`, `direct_scanout_eligible` |

## Tests

Pure tests in each file (no GPU): `shm_span` (3), `clamp_damage` (6), dma-buf conversion through a real Smithay
`Dmabuf` built on a temporary file (3), the ring transitions (9, including a 30-frame triple-buffering run) and the
scanout check (9). The GPU path (`Device::import_dmabuf`, `replace_region`) is not covered here; it needs a GPU run
on Midget or Titan.

Expected values were worked out by hand: for example XRGB8888 is `0x34325258` and ARGB8888 `0x34325241`, and
`shm_span(10, 4, 64) = 3 * 64 + 40 = 232`.

## What is left (todo_left)

- The shm format has no channel in the signature: X (XRGB8888) buffers are uploaded as BGRA8, so their alpha byte
  is whatever the client wrote. The caller (or the compositor's draw) must ignore alpha for them, or the contract
  should carry the format into `upload_shm`.
- `from_gbm` does not hold the GBM buffers; `drm.rs` must keep them with the KMS framebuffers, in the same order.
- No import-side fence wait: client `wp_linux_drm_syncobj` acquire points belong to the compose owner, and the
  implicit fence is not waited on here.
- `import_dmabuf` does not validate that a cached key still refers to the same client buffer; `forget` on
  `wl_buffer` destroy is required from the handlers.
- Damage rectangles are uploaded one by one; a region merge would save waits for many small rectangles.
- Not run: `cargo test` (this wave runs `cargo check` only); the tests above are verified by hand and must be run
  by the integrator.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. `cargo check --offline -p bifrost-wm --tests` in `springboard/` (passes in this wave).
2. On a Midget build with `BIFROST_WM_RENDERER=vulkan`, start one SDK app with a dma-buf backing store, and check in
   the window server log that `import_dmabuf` runs once per `wl_buffer`, not per frame.
3. Resize an shm client and check that the texture is recreated only on the size change.
4. Compare the output with `bifrost pc midget screen` against the CPU renderer: the content must match.
5. With `BIFROST_WM_STATS=1`, check under load that `free_count` never stays at zero while frames are flipping.
