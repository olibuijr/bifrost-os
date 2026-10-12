# G1: GPU memory, buffers and shared events (`bifrost-gpu`)

Owner: wave G1 agent `gpu-memory-sync`. Contract: `docs/graphics-architecture.md`, "bifrost-gpu". Files:
`springboard/gpu/src/memory.rs`, `springboard/gpu/src/sync.rs`, and the `new_buffer` / `new_shared_event` factories in
`springboard/gpu/src/device.rs`.

## What is implemented

### Buffers (`memory.rs`)

- `Device::new_buffer(len, usage)` rejects a zero length. Buffers under 256 KiB (`SUBALLOC_LIMIT`) are
  sub-allocated; larger buffers get their own `VkBuffer` and `VkDeviceMemory`.
- Host-visible (`STAGING` or `MAPPED`): memory is `HOST_VISIBLE | HOST_COHERENT`, mapped whole for the buffer's
  lifetime, so `write` and `read` are memcpy with no flush. Device-local (everything else): `DEVICE_LOCAL`, no CPU
  address, so `write` and `read` return `Error::Invalid`.
- `Buffer::write(offset, bytes)` and `Buffer::read(offset, len)` check the range (overflow included) before touching
  memory.
- Every buffer is created with `TRANSFER_SRC | TRANSFER_DST` plus its kind flags (vertex, index, uniform, storage).
- Dropping the last handle returns a sub-allocated range to its block, or destroys a dedicated buffer and frees its
  memory.
- Memory type choice uses the existing pure `memory_type_index`. Host memory with no matching type is
  `Error::Unsupported`. Device-local falls back to any allowed type.

### Block sub-allocator

- A pool is one block list per (device, host-visibility). Pools live in a registry of weak handles, and each buffer
  holds its pool, so a block lives while any buffer of its pool does.
- A block is `BLOCK_SIZE` (4 MiB): one `VkBuffer` over the whole block, with the union of all kind flags, plus its
  memory. Every sub-allocation is a range of that buffer, so `Buffer` offsets are plain offsets into it.
- Offsets are aligned to `max(256, requirements.alignment)`. 256 is the largest `min*OffsetAlignment` limit the spec
  allows, so a binding at any sub-allocated offset is legal.
- `SubAllocator` is pure bookkeeping (first-fit over a `BTreeMap` free list, neighbours merged on free, no Vulkan).
  It is host-tested, including the coalescing and alignment cases worked by hand.

### Shared events (`sync.rs`)

- `Device::new_shared_event()` makes a timeline semaphore starting at 0.
- `signaled_value()` reads the counter (`vkGetSemaphoreCounterValue`).
- `wait(value, timeout_ns)` uses `vkWaitSemaphores`. `Ok(true)` is reached, `Ok(false)` is a timeout, and other
  codes map through `Error::from`.
- `signal(value)` uses `vkSignalSemaphore`. Vulkan rejects a value that is not greater than the current one.
- `export_sync_file(value)`: creates a binary semaphore with `SYNC_FD` export, submits (`vkQueueSubmit2`) a wait on
  the timeline value and a signal of the binary semaphore, exports it with `vkGetSemaphoreFdKHR` (`SYNC_FD`), then
  destroys the binary semaphore. The fd is wrapped in an `OwnedFd`. A negative fd is `Error::Unsupported`, reported
  rather than faked.
- `QUEUE_LOCK` (`sync::lock_queue()`) serialises queue submissions made here. Vulkan requires external
  synchronisation of a queue, so `command.rs` must take the same lock for its own `vkQueueSubmit2`.
- `SharedEvent` `Drop` destroys the semaphore. In-flight submissions keep a clone (`CommandBuffer::waits`).

## Mapping to the contract

| Contract item | Where |
|---|---|
| `Buffer` (vertex, uniform, storage, staging), mapped when host-visible | `memory.rs` `new_buffer`, `Buffer::write/read` |
| dedicated allocations for textures and buffers, block sub-allocator for small buffers | `memory.rs` `Pool`, `SubAllocator` |
| `SharedEvent` = timeline semaphore with sync_file import/export | `sync.rs` (export only; import is left, see below) |
| `Submission::sync_file` | owned by `command.rs` (`gpu-command`); it can call `sync::lock_queue()` and the export path above |

## What is left (todo_left)

- sync_file import (`VK_KHR_external_semaphore_fd` import of a `SYNC_FD`) for client acquire fences. Not in the
  skeleton's public API, so it is not added here.
- Lifetime across GPU use: `Drop` for a buffer does not wait for in-flight work. The command layer must keep every
  resource of an in-flight submission alive (`CommandBuffer::keep`) until the fence signals. This is the contract's
  rule, and this module relies on it.
- Export ordering: a sync_file export submits a queue wait on the timeline value. If later work on the same queue
  signals that value, the wait deadlocks the queue. Callers must signal from another queue or from earlier
  submissions. This is documented on the module and needs a check with the validation layers on Midget.
- Export of a semaphore that already has its value: the driver may return no fd (-1), which is reported as
  `Error::Unsupported`. Callers should check `signaled_value()` first. The exact driver behaviour needs confirming on
  Midget.
- Destroying the binary semaphore right after the export is the usual pattern, but the payload transfer should be
  confirmed with the validation layer on Midget.
- Other agents' code paths that call `vkQueueSubmit2` must take `sync::lock_queue()`.
- `device.rs` `Device::open` (`gpu-device`) must enable `timelineSemaphore` and `synchronization2`, or the timeline
  semaphore creation and `queue_submit2` fail.

## Verify

Host (pure, no GPU):

```
cd springboard
CARGO_TARGET_DIR=$PWD/target-agents cargo check --offline -j 2 -p bifrost-gpu --tests
```

The pure tests (`SubAllocator`, `memory_type_index`, range checks, usage mapping, wait-outcome mapping, fd wrapping)
run as `bifrost-gpu` unit tests. The `gpu-tests` feature adds buffer round trips, device-local refusal, dedicated
buffers, and the timeline wait and reach tests. Run them only where a Vulkan device exists.

Midget (Intel HD 530, Mesa ANV, Vulkan 1.4), once `Device::open` is in place:

```
cargo test --offline -p bifrost-gpu --features gpu-tests
```

Run it on Midget with `VK_LOADER_DEBUG=error` and the Khronos validation layer enabled, and expect no validation
errors for buffer creation, memory mapping, timeline wait and signal, and the sync_file export.
