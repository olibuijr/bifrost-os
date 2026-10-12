# G1 gpu-device: Vulkan instance, device and queue

Owner: `gpu-device`. Contract: `docs/graphics-architecture.md`, "bifrost-gpu". Code: `springboard/gpu/src/device.rs`.

## What is implemented

`Device::open(render_node)` (`springboard/gpu/src/device.rs`):

1. Stats the render node (`/dev/dri/renderD128`) when given and splits its `st_rdev` into DRM major and minor
   (`dev_numbers`, glibc's `major()`/`minor()` layout). No `unsafe` in this step.
2. Loads the system loader with `ash::Entry::load()`. A missing loader is `Error::Unsupported("Vulkan loader: ...")`.
3. Requires instance API 1.3 (`try_enumerate_instance_version`; a 1.0 loader is `Unsupported`).
4. Creates the instance with `API_VERSION_1_3`. The layer `VK_LAYER_KHRONOS_validation` is enabled only when
   `BIFROST_GPU_VALIDATE=1` (exact value `1`). If the layer is not installed, that is `Unsupported`.
5. Enumerates physical devices and probes each one: name, driver name and info (driver properties), API version,
   DRM render node (`VK_EXT_physical_device_drm`, chained only when the device advertises it), extensions, the
   features `dynamicRendering`, `synchronization2` and `timelineSemaphore`, and the queue families.
6. Chooses a device (`choose_device`, pure):
   - with a render node: the device whose DRM render node matches. A node with no device, or a device that lacks
     something, is `Unsupported` naming the node or the missing items. The compositor must scan out on the GPU it
     renders on, so there is no silent fallback to another GPU.
   - without one: the first usable device, discrete before integrated before virtual before CPU.
   A device is usable when it has API 1.3, all required extensions and features, and a graphics+compute queue
   family with at least one queue.
7. Creates the logical device with the required extensions
   (`VK_KHR_external_memory_fd`, `VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier`,
   `VK_KHR_external_semaphore_fd`, `VK_EXT_physical_device_drm`, `VK_EXT_queue_family_foreign`), plus
   `VK_KHR_push_descriptor` when the driver has it, one queue (priority 1.0) and the three features above enabled.
8. Fills `DeviceInfo` (name, driver, API, render node, push descriptors) and keeps the memory properties.
9. `Device::wait_idle` calls `vkDeviceWaitIdle`.
10. `Drop for Inner` waits idle, destroys the device, then the instance. `InstanceGuard` destroys the instance if
    device selection fails.

Errors: `Error::Unsupported` names what is missing. Vulkan result codes go through `Error::from`.

`unsafe` appears only in `device.rs` and each block has a `SAFETY` comment.

## Not in this part

Other `todo!()` bodies in `device.rs` belong to other owners and are unchanged: `new_command_queue` (gpu-command),
`new_texture` and `import_dmabuf` and `supported_dmabuf_formats` (gpu-texture), `new_buffer` (gpu-memory),
the pipeline and sampler constructors (gpu-pipeline), `new_shared_event` (gpu-sync).

The `Inner` fields `entry`, `physical`, `queue`, `queue_family` and `memory_properties` are filled but not read
yet. Their readers come with the other owners' modules, so the dead-code warnings go away then.

## Tests

- Pure logic, in `device.rs` `mod tests` (hand-worked values, no GPU needed, 16 tests):
  API word split, device number split (including the high bits of major and minor), the validation flag,
  missing extensions and features, the graphics+compute queue family, the NUL-terminated name, driver text join,
  device rank, and `choose_device` (integrated over an unusable discrete GPU, render-node selection, the named
  errors, Vulkan 1.2 rejection).
- `gpu-tests` feature (`mod gpu_tests`): `open(None)` reports a name and API >= 1.3, `wait_idle` succeeds; and
  `open(/dev/dri/renderD128)` reports the same node that `stat` shows.

Verified here: `CARGO_TARGET_DIR=.../springboard/target-agents cargo check --offline -j 2 -p bifrost-gpu --tests`
passes with no error. `python3 tools/check-names.py` is clean. Tests were not run in this round, per the
agent rules (`cargo test` is not allowed for this wave).

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

Midget is the test machine (`CLAUDE.md`). The device is Skylake GT2 (`8086:1912`), so ANV should report
`Intel(R) HD Graphics 530 (SKL GT2)` with API 1.4.

1. On the Midget, check the loader and the device first:
   `vulkaninfo --summary` lists `deviceName` and `driverVersion`, and `apiVersion` 1.4.
2. Run the `gpu-tests` on the device (with the render node, `/dev/dri/renderD128`):
   `cargo test --offline -p bifrost-gpu --features gpu-tests device::gpu_tests -- --nocapture`.
   Expected: both tests pass; `render_node` equals the device numbers of `/dev/dri/renderD128` (226:128 on Midget).
3. Validation: `BIFROST_GPU_VALIDATE=1` with the `vulkan-validation-layers` package installed must open the device
   with no validation error. Without the package the open fails with `Unsupported` naming the layer.
4. Negative check: `Device::open(Some("/dev/dri/renderD129"))` on a single-GPU machine must fail with
   `no Vulkan device for render node 226:129`.

## Todo left

- The other owners' `todo!()` bodies listed above.
- No `VK_KHR_push_descriptor` use yet; the flag is only recorded in `DeviceInfo`.
- The validation layer's messages go to the layer's default output; a debug messenger that routes them into the
  `bifrost-log` registry is not wired.
- The device-lost path (`Error::DeviceLost`) is mapped from `vk::Result` but not yet exercised on Midget.

## Contract changes

None. Public signatures in `device.rs` are unchanged; `Inner` gained a `Drop` impl and the module gained private
helpers and a `Candidate` type.
