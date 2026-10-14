# Sharing pixel buffers with the GPU

Matrix row: `docs/parity-matrix/data/model-i-o.json`, feature "Sharing buffers with the GPU framework". Module:
`springboard/sdk/src/buffer_share.rs` (`bifrost_app::buffer_share`). The device types are in `springboard/gpu/src/`
(`dmabuf.rs`, `format.rs`). Tests: `springboard/sdk/tests/parity_sdk-rest-15.rs`.

## What it mirrors

The reference framework hands buffers to its GPU framework (Metal, through the interop of MetalKit, GLKit and SceneKit).
BifrOSt's counterpart is the dma-buf path: a buffer is a fixed layout, and a device imports it with a DRM fourcc and a
modifier. Only the layout and the choice of import format are typed here.

- `BufferLayout::new` and `BufferLayout::with_stride` validate width, height (at most 16384), the row stride (at least one
  row, at most `MAX_STRIDE`, a whole number of pixels) and the pixel format.
- `byte_len` and `check_bytes` give the exact buffer length and reject any other length.
- `fourcc` gives the DRM code of the pixel format (`bifrost_gpu::format::PixelFormat::fourcc`).
- `choose_import_format` picks, from the list a device reports (`DmaBufFormat`), the first pair with the layout's fourcc,
  or the named modifier. It reports `NoImportFormat` when the device has neither.

The device list is an argument, so the module opens no device.

## What needs the lab machine

Export and import of a real dma-buf through `bifrost_gpu::Device` (`export_dmabuf`, `import_dmabuf`) and the list of
formats the lab GPU supports (`supported_dmabuf_formats`). None of that is wired here.

## Remaining gaps

- No 3D consumer: the Linux GL path draws 2D quads only, so there is nothing to share a mesh buffer with.
- Model I/O mesh buffers (`MDLMeshBuffer`) are not connected to this layout.
- No check of a `DmaBuf` value (plane count, plane strides) is made: constructing one needs file descriptors, which the
  headless tests do not create.
