# G1 `gpu-texture`: textures and dma-bufs in bifrost-gpu

Wave G1 of the graphics core (`docs/graphics-architecture.md`, "bifrost-gpu"). This part owns
`springboard/gpu/src/texture.rs`, `springboard/gpu/src/dmabuf.rs` and the texture and dma-buf methods of
`device.rs`: `new_texture`, `import_dmabuf`, `supported_dmabuf_formats`.

## What is implemented

`texture.rs`

- `Device::new_texture` (`texture::create`). Makes a 2D image with its own dedicated `VkDeviceMemory` and a colour
  view.
  - Plain textures use `OPTIMAL` tiling and no external memory.
  - Shared textures (`exportable`, or a non-empty `modifiers` list) use `DRM_FORMAT_MODIFIER_EXT` tiling. The
    candidate modifiers are the descriptor's list (or `LINEAR` when the list is empty), filtered to those the device
    reports with the features the usage needs (`pick_modifiers`), in preference order, through
    `VkImageDrmFormatModifierListCreateInfoEXT`.
  - Exportable memory is allocated with `VkExportMemoryAllocateInfo` (`DMA_BUF_EXT`) and a dedicated allocation.
  - The memory type is device-local when possible.
  - The layout starts `UNDEFINED`.
- `Texture::replace_region`: validates the region and the stride (`pack_rows`), copies the rows tightly into a
  host-visible staging buffer, and records a one-shot command buffer on the device queue. It transitions to
  `TRANSFER_DST_OPTIMAL`, copies, transitions to the natural layout for the usage, submits, and waits on a fence.
- `Texture::read_region`: the same path in reverse (`TRANSFER_SRC_OPTIMAL`, buffer copy, wait). Returns tight pixels.
- `Drop` for the shared `Inner`: destroys the view, the image, then frees the memory. Each handle is checked for null,
  so a constructor that fails part way cleans up whatever was created.
- Pure helpers, all unit-tested without a GPU: `usage_flags`, `required_features`, `pick_modifiers`, `natural_layout`,
  `validate_region`, `pack_rows`, `color_range`, `barrier`.

`dmabuf.rs`

- `Texture::export_dmabuf` (`dmabuf::export`). Reads the modifier the driver chose
  (`vkGetImageDrmFormatModifierPropertiesEXT`). It exports one fd for the memory object
  (`vkGetMemoryFdKHR`), duplicates it for each memory plane, and reads each plane's offset and pitch with
  `vkGetImageSubresourceLayout` on the `MEMORY_PLANE_i` aspects.
- `Device::import_dmabuf` (`dmabuf::import`). Checks that the fourcc maps to a `PixelFormat`, that the (format,
  modifier) pair is sampleable, and that the plane count matches the modifier.
  - It creates the image with `VkExternalMemoryImageCreateInfo` (`DMA_BUF_EXT`) and
    `VkImageDrmFormatModifierExplicitCreateInfoEXT` (the modifier and the plane layouts).
  - It wraps the first plane's fd in `VkImportMemoryFdInfoKHR`, with a dedicated allocation.
  - Memory types are the intersection of the image's and the fd's (`vkGetMemoryFdPropertiesKHR`).
  - An import is marked foreign. Its layout is tracked as `GENERAL`, and its usage is what the modifier's format
    features allow (`import_usage`).
- `Device::supported_dmabuf_formats` (`dmabuf::supported_formats`). Walks the shareable pixel formats and lists each
  modifier that reports `SAMPLED_IMAGE`, using `vkGetPhysicalDeviceFormatProperties2` with
  `VkDrmFormatModifierPropertiesListEXT` (a count call, then a list call).

## How it maps to the contract

| Contract item | Where |
|---|---|
| `Texture` from a `TextureDescriptor` (size, format, usage, exportable, modifiers) | `texture::create` |
| `Device::import_dmabuf(&DmaBuf)` | `dmabuf::import` |
| `Texture::export_dmabuf()` | `dmabuf::export` |
| `Texture::replace_region` through a staging buffer | `Texture::replace_region`, `transfer` |
| Shared memory is dedicated and exportable as DMA_BUF | `texture::create` (`ExportMemoryAllocateInfo`) |
| Queue family foreign for imports | `transfer` acquires from `QUEUE_FAMILY_FOREIGN_EXT` for foreign images |
| `supported_dmabuf_formats` from `VkDrmFormatModifierPropertiesListEXT` | `dmabuf::supported_formats` |

No public signature in the skeleton changed. The three `device.rs` bodies only delegate to the modules above.

## Decisions and known limits

- One memory object per texture. Planes of a modifier image share it. Disjoint multi-memory images are not made.
- Imported planes must all come from one dma-buf (the first plane's fd is used for the memory import). Clients send
  one buffer per plane with the same fd in practice, which this handles.
- One-shot copies create a transient command pool and fence per call, then wait. This is simple and correct, and is
  the cost to remove first (see `todo_left`).
- Imported textures are sample-only unless the modifier's format features allow transfers or attachments.
  `read_region` and `replace_region` return `Error::Invalid` when the usage flag is missing.
- Imports assume the producer leaves the image in `GENERAL` and acquire it with an ownership barrier from the foreign
  family on each one-shot use. Verifying the contents survive the acquire on Mesa ANV is part of the Midget check.
- A queue submission lock lives in this module (`SUBMIT_LOCK`). The command module must take the same lock when it
  submits to the same queue, or move the lock into `device::Inner`.

## todo_left

- Replace the per-call transient pool with a pool owned by the device or command queue, and reuse one staging buffer
  for atlas and shm uploads (today each call allocates and frees one).
- Move `SUBMIT_LOCK` into `device::Inner` (it is per device, not global) and have `gpu-command` take it.
- Release foreign-owned images back to the producer when a frame hands them to a client. That belongs to the command
  module's submission path; this part only acquires.
- Dimension and format limit checks against `maxImageDimension2D` and the format's image format properties
  (`vkGetPhysicalDeviceImageFormatProperties2`); today the driver rejects oversize images at create time.
- Disjoint multi-plane images (separate memory per plane, for planar YUV from video decode).
- Multi-fd imports for planes that come from different dma-bufs.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

Host, without a GPU (pure logic):

```
cd springboard
CARGO_TARGET_DIR=$PWD/target-agents cargo check --offline -p bifrost-gpu --tests
```

On Midget, with the device implemented (`gpu-device` and `gpu-command` merged), run the GPU tests:

```
cd springboard
cargo test --offline -p bifrost-gpu --features gpu-tests texture
```

The four `gpu_tests` in `texture.rs` cover:

1. Upload then read of a full 4x4 texture (tight pixels, row padding dropped).
2. A sub-region upload leaves the rest of the texture alone.
3. Export to a dma-buf, import it back, and read the same pixels (round trip through the kernel).
4. `supported_dmabuf_formats` lists `ARGB8888` with `DRM_FORMAT_MOD_LINEAR`.

Check the driver's modifier list on Midget with `vulkaninfo --json` (`VK_EXT_image_drm_format_modifier`), and confirm
`VK_EXT_external_memory_dma_buf`, `VK_KHR_external_memory_fd` and `VK_EXT_queue_family_foreign` are enabled.

Before a row in the parity matrix is marked `parity`, the round trip must work on Midget, not only on the host.
