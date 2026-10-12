# Parity wave 4: Metal (GPU) pure models

Framework: Metal, https://developer.apple.com/documentation/metal (matrix slug `metal`, status `os`, 31.2% coverage
before this wave).

BifrOSt has no app-facing GPU device, so this wave builds the parts of the GPU API that can be modelled in pure Rust
with no hardware access. Each module is a CPU model in `springboard/ui/src` (`bifrost_ui`). None of them creates a GPU
object, binds a device or submits work. They are the checks and bookkeeping that a future SDK GPU API and the
compositor's own queue can share. Nothing is wired into apps or the shell yet.

The tests are written with values worked out by hand from each rule. They have not been run: this wave does not build.

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/texture_descriptor.rs` | `MTLTextureDescriptor` (`texture2DDescriptor(pixelFormat:width:height:mipmapped:)`, `textureCubeDescriptor(pixelFormat:size:mipmapped:)`, `textureType`, `pixelFormat`, `width`, `height`, `depth`, `mipmapLevelCount`, `sampleCount`, `arrayLength`), `MTLTextureType`, `MTLPixelFormat` (subset) | Kind (2D, 3D, cube), pixel format size, full mip count, validation (size limit, cube squareness, array rules, mip range, sample count 1/2/4/8, multisample needs one 2D level), byte count of the mip chain times faces, samples and array length, row padding to an alignment. |
| `springboard/ui/src/compute_dispatch.rs` | `MTLSize`, `MTLComputeCommandEncoder.dispatchThreadgroups(_:threadsPerThreadgroup:)` and `dispatchThreads(_:threadsPerThreadgroup:)` (the official page for the first is cited) | Three-axis size, threadgroup validation against a thread limit, grid-to-threadgroup rounding up on each axis, and a serial CPU reference walk of every in-grid thread (x fastest, then y, then z, group by group). |
| `springboard/ui/src/command_buffer.rs` | `MTLCommandBuffer` (`enqueue()`, `commit()`, `addScheduledHandler(_:)`, `addCompletedHandler(_:)`, encoder factory methods) and `MTLCommandQueue` submission order | Lifecycle state machine Recording, Committed, Scheduled, Completed. Enqueue once while recording. One pass open at a time, ended passes kept in recorded order, no commit with an open pass, forward-only queue steps. |

Existing modules were extended in place or left alone: `springboard/ui/src/blit.rs` (blit passes, already a partial
row) and `springboard/ui/src/drawable.rs` (drawable colour, depth and sample formats) were not changed.

## API names and REA evidence

| Official page | URL | REA evidence id |
|---|---|---|
| Metal (landing) | https://developer.apple.com/documentation/metal | `ev_cb8ceaec342920fe6bc28d4386e9714e787a7e68e329a329caa8b599556c9b32` |
| MTLTextureDescriptor | https://developer.apple.com/documentation/metal/mtltexturedescriptor | `ev_4946035650f77a033a799ba0cdd6baad9fdf22423881deb2808db304acb842c3` |
| MTLComputeCommandEncoder.dispatchThreadgroups(_:threadsPerThreadgroup:) | https://developer.apple.com/documentation/metal/mtlcomputecommandencoder/dispatchthreadgroups(_:threadsperthreadgroup:) | `ev_ae68cc575586345d530f1ae047dd74b57d3376283a809614054827fc8af78613` |
| MTLCommandBuffer | https://developer.apple.com/documentation/metal/mtlcommandbuffer | `ev_fdd0aa9ee6b28bda0f772b7891109620cfd6a55f0caedc8c3af89980dada801e` |

DocC facts used (from `python3 -I tools/parity/docc.py`): MTLTextureDescriptor has the three factory methods listed
above; dispatchThreadgroups is "a compute dispatch command using a grid aligned to threadgroup boundaries"; MTLCommandBuffer
has `enqueue()` (reserves the next place in the queue), `commit()` (submits to run on the GPU), and the scheduled and
completed handlers; MTLSize has width, height and depth.

## Matrix rows closed

Status moves to `partial` for each row. The lead updates `docs/parity-matrix/` centrally.

| Framework slug | Feature (as written in `metal.json`) | New status | BifrOSt path |
|---|---|---|---|
| metal | Buffers and textures | partial | springboard/ui/src/texture_descriptor.rs |
| metal | Compute passes | partial | springboard/ui/src/compute_dispatch.rs |
| metal | Command queue and work submission | partial | springboard/ui/src/command_buffer.rs |

Not closed, and why: buffers (no buffer size or storage model beyond textures), shaders, GPU counters and HDR presentation
still need a GPU or a system presenter change. Blit, machine learning, ray tracing and indirect encoding were left as
they are.

## Checks run

- `python3 tools/check-names.py --summary`: 0 hits.
- Not run: cargo, rustc, make, tests (per the wave rules).
