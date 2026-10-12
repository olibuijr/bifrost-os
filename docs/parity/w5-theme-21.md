# Wave 5, theme-21: DiskImageKit error types

## What was built

Error types for the disk image model in `springboard/ui/src/disk_image.rs` (bifrost-ui). The module is pure: no file is
created, opened or resized, and no app can reach it yet.

- `ImageError::UnsupportedFormat { name }` is new. `Format::parse(name)` accepts `raw` in any case and returns that error for any other name.
- `ImageError::type_name()` maps each value to the official error type name. A zero or negative block count is
  `InvalidBlockCountError`, a damaged or partial-block file is `CorruptedImageError`, and an unknown format is
  `UnsupportedFormatError`. Size overflow and read-only refusals have no official type and return `None`.
- Display text for the new variant: `image format NAME is not supported`.
- Tests: 5 new behaviour tests in the module's `tests` block. They cover format parsing, the official type names,
  the `None` cases, and the corrupted-length check at 4 KB block size.

## Official API names mirrored

- `DiskImageKit` framework page: the Errors section lists `IncompatibleStackingError`, `InvalidBlockCountError`,
  `CorruptedImageError` and `UnsupportedFormatError`.
- `InvalidBlockCountError`: "The block count specified for the disk image is invalid (zero or negative)."
- `CorruptedImageError`: "The disk image is corrupted or contains invalid data."
- `UnsupportedFormatError`: "The disk image format isn't supported."
- `IncompatibleStackingError` is not built. Layered stacking (`StackedImage`, `appending(_:)`) has no BifrOSt
  counterpart yet, so the row stays partial.

## REA evidence

- `ev_0bca29540f4436314d59b18e1cbb5638e8f5e12fdd783262924de94ed5df45ee`: DiskImageKit framework page
- `ev_5b83ab8db2a26b98a69104cd7355aba293c6e9892548f5eeca6e63969a51fe62`: UnsupportedFormatError page
- `ev_3f5a71ed6f806ae63db99810d0d29155f77939be77f9fe1b434d75a3504f6c09`: CorruptedImageError page
- `ev_b320a2c3c0bee06d5a56be09d167bffb5e9d55d340e0c2428f78a4ea031dca6d`: InvalidBlockCountError page

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| diskimagekit | Error types | Partial | springboard/ui/src/disk_image.rs |

## Not closed

- `Inspect image properties` (diskimagekit): the UI model has accessors, but there is no app-facing properties API. Left as a gap.
- `IncompatibleStackingError`: stacking is not modelled. This stays `n/a` or a gap until layers are built.
- The core copy at `springboard/core/src/disk_image.rs` has its own `ImageError` with `UnsupportedFormat`. It was not changed, because that module is a second copy and the UI model is the one with the Apple-mirrored names.
