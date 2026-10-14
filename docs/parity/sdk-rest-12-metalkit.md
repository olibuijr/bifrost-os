# Parity sdk-rest-12: GPU drawable setup and sprite batching (MetalKit rows)

Mirrors the drawable side of the GPU view framework (device and drawable setup, and per-frame draw batching for 2D draws).

## What is wired

- `springboard/ui/src/drawable_setup.rs` (`DrawableSetup::validate`): checks a render view's size against the device's
  largest side, refuses zero sizes and malformed render node paths, and returns the four-character format code the
  presenter allocates with (`XB24` opaque, `AB24` with alpha, the same codes as `springboard/src/gpu/wl.rs`).
- `springboard/ui/src/sprite_batch.rs` (`SpriteBatch::push_quad`): appends quads to one vertex list, joins solid quads to
  the textured run they follow, starts a run on a texture change, and reports `QuadError::Full` at 4096 quads.
- Tests: `springboard/ui/tests/parity_sdk-rest-12.rs`, headless, no device and no frame drawn.

## What needs the lab machine

- The presenter still allocates buffers in `springboard/src/gpu/wl.rs` and `springboard/src/gpu/batch.rs`. Those stay
  internal to the shell; the models above do not yet feed them.
- No app receives a drawable or a device handle. Binding a `DrawableSetup` to a real GBM buffer, and checking the
  maximum side the GPU reports, needs the lab machine.
