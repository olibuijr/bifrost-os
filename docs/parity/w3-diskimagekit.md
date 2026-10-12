# Wave 3: DiskImageKit (RAW disk image model)

## What was built

`springboard/ui/src/disk_image.rs` (`bifrost_ui::disk_image`), a pure model with no file I/O:

- `BlockSize` (512 bytes or 4 KB) with `bytes()` and `from_bytes()`.
- `Format` (RAW only). A sparse format is not modelled.
- `OpenMode` (read-only or read-write).
- `RawCreation::new(block_count, block_size)` validates the configuration, rejects zero, negative and
  overflowing sizes, and reports `size()`.
- `DiskImage::create(path, config)` and `DiskImage::open_raw(path, length, block_size, mode)`, which rejects a
  zero or partial-block length. Accessors: `path`, `format`, `block_size`, `block_count`, `size`, `open_mode`.
- `DiskImage::truncate(new_count)` grows or shrinks the block count and refuses read-only images. The filesystem
  inside the image is not touched.
- `ImageError` (`InvalidBlockCount`, `SizeOverflow`, `ReadOnly`, `CorruptedImage`) with a `Display` impl.

The module is registered once in `springboard/ui/src/lib.rs`. It has 8 behaviour tests with hand-worked values.

## Official API names mirrored

Source: https://developer.apple.com/documentation/diskimagekit (DocC, macOS 27.0)

- `DiskImage`, `init(creating:)`, `init(opening:)`, `truncate(blockCount:)`, `blockCount`, `blockSize`
  (512 bytes or 4 KB), `format`, `openMode`, `size`, `url`
- `RAWCreationConfiguration`, `DiskImage.CreationConfiguration`
- `DiskImage.BlockSize`, `DiskImage.Format`, `DiskImage.LayerType` (LayerType not modelled)
- `InvalidBlockCountError` (zero or negative count), `CorruptedImageError`, `UnsupportedFormatError`

Not mirrored: ASIF creation, stacked images and layers, `appending(_:)`, `IncompatibleStackingError`, the
Virtualization storage integration, and image properties beyond those listed. Apple's `url` is a path string here.

## REA evidence

- DiskImageKit landing page: `ev_a8f68c0806cca744e54b9ee16e1ad7c49d1f4e2e7979035057a2645ecda9d5ac`
  (rendered via `tools/parity/rea-page.sh`).

## Matrix rows closed (status partial)

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| DiskImageKit | Create an empty disk image (RAW) | partial | springboard/ui/src/disk_image.rs (RawCreation, DiskImage::create) |
| DiskImageKit | Open an existing disk image | partial | springboard/ui/src/disk_image.rs (DiskImage::open_raw) |
| DiskImageKit | Block size and format values | partial | springboard/ui/src/disk_image.rs (BlockSize, Format, OpenMode) |
| DiskImageKit | Truncate or extend an image | partial | springboard/ui/src/disk_image.rs (DiskImage::truncate) |

## Still open

- Writing or reading the image file, and mounting it as a block device (needs a system-layer loop device service).
- Files app action for ISO or image mount and eject.
- Inspect image properties beyond the accessors above, error types for stacking, and the stacked and ASIF rows.
