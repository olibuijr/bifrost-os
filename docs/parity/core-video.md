# Core Video

Branch `parity/team-j-core-video`, based on `arm`. Pure Rust, std only, no unsafe, no hardware access. Nothing is
wired into apps or the shell. The code has not been built or tested on Midget or the VM; the lead builds and checks
it after merge.

Reference: https://developer.apple.com/documentation/corevideo

## What is app-facing

**`springboard/sdk/src/pixel_buffer.rs`** (`bifrost_app::pixel_buffer`)

- `PixelBuffer`: CPU-side buffer in a format of `bifrost_render::pixel_format`. Locks nest; bytes are readable under any
  lock and writable only under a read-write lock. A read-write unlock advances `revision()`, which caches compare.
- Colour metadata: `attachments()`, `set_attachments()` (the `Attachments` model of
  `bifrost_render::buffer_attach`: primaries, transfer function, YCbCr matrix, clean aperture), `color_space()` and
  `clean_rect()`. Attachments do not change the pixels, so they do not change the revision.
- `PixelBufferPool`: buffers of one format and size, reusing `bifrost_render::buffer_pool`.

**`springboard/sdk/src/display_link.rs`** (`bifrost_app::display_link`): unchanged in this branch. Refresh period and
time stamps from the app's own ticks. No per-display callback.

**`springboard/sdk/src/texture_cache.rs`** (`bifrost_app::texture_cache`)

- `TextureCache::texture_for(key, &buffer, make)`: returns the texture cached for `key` while the buffer's revision,
  format and size match. Otherwise it calls `make`, which creates the GPU texture and returns its name. A replaced
  name comes back in `TextureUpdate::replaced` so the caller can delete it.
- `remove(key)`, `flush()` (returns every name to delete), `len`, `is_empty`, `hits`, `misses`.
- The model never touches a GL context. The caller's function does that.

## Official API names mirrored

- `CVImageBufferGetColorSpace`, `CVImageBufferGetCleanRect`, `CVBufferSetAttachment`, `CVBufferCopyAttachments`:
  `PixelBuffer::color_space`, `clean_rect`, `set_attachments`, `attachments`.
- `CVOpenGLESTextureCacheCreateTextureFromImage`, `CVOpenGLESTextureCacheFlush`, `CVOpenGLTextureGetName`:
  `TextureCache::texture_for`, `flush`; the name is the value `make` returned.

## Not mirrored

- `CVMetalTextureCache*`: Metal has no Linux counterpart in BifrOSt.
- `CVPixelBufferCreateWithIOSurface`, `CVPixelBufferGetIOSurface`: IOSurface is Apple-only; shared memory with the
  compositor stays a system feature (`springboard/src/gpu/`).
- `CVReturn` result codes: the crate uses `Result` with its own error enums.
- `CVDisplayLink` output callback and `CVDisplayLinkCreateWithActiveCGDisplays`: see `docs/bifrost-ui.md`.

## Matrix rows

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-video | Image buffer attachments and colour metadata | partial | springboard/render/src/buffer_attach.rs, springboard/sdk/src/pixel_buffer.rs |
| core-video | Texture caches for GPU import | partial | springboard/sdk/src/texture_cache.rs |

The matrix data is `docs/parity-matrix/data/core-video.json`. The other core-video rows (pixel buffers, pool, display
timing, pixel formats) were partial before this branch, and their notes now point here.

## Gaps that remain

- Decoded pictures (`springboard/src/media/mod.rs`, `DmaPicture`) carry no colour metadata, so the decoder does not fill
  the attachment model yet.
- The texture cache is not connected to the presenter's GL context. An app cannot yet import a buffer as a texture on
  screen.
- Pixel buffers are not shared with the presenter's GBM buffers.

## Tests

Host unit tests (`#[test]`) were added for `PixelBuffer` attachments and revision (`pixel_buffer.rs`) and for the
texture cache (`texture_cache.rs`). They have not been run in this branch.
