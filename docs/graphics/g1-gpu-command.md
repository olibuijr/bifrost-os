# G1 gpu-command: command queues, command buffers, encoders

Owner: wave G1 agent `gpu-command`. Contract: `docs/graphics-architecture.md`, "bifrost-gpu". Files:
`springboard/gpu/src/command.rs` (all of it) and `springboard/gpu/src/device.rs` (`new_command_queue` only).

## What is implemented

- **CommandQueue**: owns one command pool (created on first use, `RESET_COMMAND_BUFFER`) on the device's graphics
  queue family. `Device::new_command_queue` returns a queue; clones share the pool.
- **command_buffer()**: allocates one primary buffer and begins it with `ONE_TIME_SUBMIT`. The buffer returns to the
  pool when its submission completes, or when an uncommitted buffer is dropped (a `Retire` guard in `keep`).
- **Render passes**: `render_encoder` moves the target to `COLOR_ATTACHMENT_OPTIMAL` and begins dynamic rendering
  (`vkCmdBeginRendering`) with the `LoadAction` (LOAD, CLEAR with premultiplied RGBA, DONT_CARE), the `StoreAction`, and
  the render area (the damage region, or the whole target). The default viewport is the whole target and the default
  scissor is the render area. Dropping the encoder, or calling `end`, ends the rendering scope.
- **Compute passes**: `compute_encoder`, `set_pipeline`, `set_bytes`, `set_texture(binding, tex, write)` (a storage
  image in `GENERAL` when `write`, a sampled image in `SHADER_READ_ONLY_OPTIMAL` otherwise), `set_sampler`, `dispatch`.
- **Render encoder**: `set_pipeline` (graphics bind point), `set_viewport`, `set_scissor`, `set_bytes` (push constants,
  zero padded to 4 bytes and capped at 128), `set_vertex_buffer`, `set_texture`, `set_sampler`, `draw`.
- **Bindings**: set 0 is pushed with `VK_KHR_push_descriptor` when `DeviceInfo::push_descriptors` is true. Otherwise a
  descriptor set is allocated per flush from a per-buffer pool (a new pool starts when one is full) and bound. A flush
  happens only when a binding or the pipeline changed since the last one.
- **Copies**: `copy_texture` (`vkCmdCopyImage2`, same format, both regions checked) and `copy_buffer_to_texture`
  (`vkCmdCopyBufferToImage2`; the stride must be a whole number of pixels and the source range is checked against the
  buffer length).
- **Layout and ownership tracking**: `CommandBuffer::track` is the single place barriers are emitted. For each texture
  it keeps, per buffer, the layout, the last stage and access, and whether it was written. One synchronization2 image
  barrier is emitted for the first use in a buffer, a layout change, a write after anything, or any use after a write.
  A read after a read in the same layout emits nothing. Layouts reach `texture::Inner::layout` only at `commit`, so an
  uncommitted buffer changes nothing.
- **Foreign images** (`Texture::inner.foreign`: dma-buf imports and exports): the first use in a buffer is a queue
  family acquire from `VK_QUEUE_FAMILY_FOREIGN_EXT`. A buffer that writes a foreign image ends with a release back to
  the foreign family, moving the image to `SHADER_READ_ONLY_OPTIMAL` so the other process or the display can sample it.
  The tracked layout becomes that layout.
- **commit**: records the releases, ends the buffer, imports each sync_file wait as a temporary binary semaphore
  (`vkImportSemaphoreFdKHR` with `TEMPORARY`; Vulkan takes the fd on success), builds timeline waits and signals from
  the `SharedEvent`s, creates a fence, and calls `vkQueueSubmit2` under a process-wide submit lock (a `VkQueue` needs
  external synchronisation). The resources stay in the returned `Submission` until its fence has signalled.
- **Submission**: `wait` (`vkWaitForFences`) and `is_complete` (`vkGetFenceStatus`). Dropping a submission waits for the
  fence, destroys it, then releases the kept resources: pool buffers return to the pool, and temporary semaphores and
  descriptor pools are destroyed.
- **Tests**: pure logic tests in `command.rs` (usage mapping, barrier rules, row length, region bounds, push constant
  padding, the release layout, thread safety). A `gpu-tests` test clears a texture and reads it back.

## Contract mapping

| Contract item | Where |
|---|---|
| `CommandQueue::command_buffer` | `CommandQueue::command_buffer` |
| `CommandBuffer::{render_encoder, compute_encoder, copy, wait, signal, commit}` | `command.rs`; `copy` is `copy_texture` and `copy_buffer_to_texture` |
| `RenderEncoder::{set_pipeline, set_viewport, set_scissor, set_bytes, set_buffer, set_texture, set_sampler, draw}` | Implemented. The skeleton names the buffer binding `set_vertex_buffer`, which is kept |
| `ComputeEncoder::{set_pipeline, set_bytes, set_texture, dispatch}` | Implemented, plus `set_sampler` from the skeleton |
| `Submission::{wait, is_complete, sync_file}` | `wait` and `is_complete` done; `sync_file` left to `gpu-sync` |
| Foreign queue acquire and release | `track`, `commit` |
| Keep resources alive until completion | `keep`, `Retire`, `Submission::drop` |

## Requirements on the other G1 files

- `pipeline.rs`: a render pipeline's set-0 layout must be created with `PUSH_DESCRIPTOR_KHR` when push descriptors are
  used (`DeviceInfo::push_descriptors`). Push-constant ranges must use `ALL_GRAPHICS` (render) or `COMPUTE` (compute),
  because `set_bytes` pushes with those stage masks.
- `texture.rs`: `Inner::layout` must start at the layout the image really holds on import (usually
  `SHADER_READ_ONLY_OPTIMAL` for a client-filled dma-buf, `UNDEFINED` for a fresh image). `foreign` is true for
  imported and exported images.
- `device.rs`: `VK_KHR_external_semaphore_fd` must be enabled (sync_file waits use it), and `Inner::queue_family` must be
  the family the queue was created from.

## Known limits (todo_left)

- `Submission::sync_file` is `todo!("gpu-sync")`. Exporting a fence needs an export-capable fence (`VK_KHR_external_fence_fd`),
  which is `gpu-sync`'s to set up.
- Buffers have no barriers. A GPU write to a buffer followed by a vertex or copy read needs a caller-side barrier; host
  writes before a commit are visible without one.
- Barriers use the stage and access of the previous and next use rather than the finest stages. The first use in a
  buffer uses `ALL_COMMANDS` to order against earlier submissions.
- Layouts are tracked in recording order. Committing buffers out of recording order breaks the layout state; the window
  server and SDK record and commit in order per queue.
- A foreign image that is only read is acquired on each use without a release (no per-image owner state is kept).
  A stricter ownership model needs an owner field in `texture.rs`.
- `draw` and `dispatch` are skipped silently when no pipeline is bound or a descriptor allocation fails. Their signatures
  return `()`, so the failure cannot reach the caller.
- Other encoder methods that return `()` (bindings, state, push constants) cannot report errors either.
- Nothing has been run on a GPU in this round (no test runs were allowed); the code has only been type-checked, and the
  `gpu-tests` test has never executed.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Host: `cd springboard && CARGO_TARGET_DIR=... cargo check --offline -j 2 -p bifrost-gpu --tests`.
2. Midget, once `gpu-device`, `gpu-texture`, `gpu-pipeline`, `gpu-memory` and `gpu-sync` have landed:
   `cargo test --offline -p bifrost-gpu --features gpu-tests clear_pass_reads_back`, with
   `VK_INSTANCE_LAYERS=VK_LAYER_KHRONOS_validation` if the layer is installed, and no validation errors in the log.
3. Exercise both binding paths: once with `DeviceInfo::push_descriptors` true, and once with the extension masked out in
   `device.rs`.
4. Check a foreign round trip: import a dma-buf, draw into it in one buffer, then sample it in a second buffer, and
   compare the pixels.
