# Disk images (slice `disk-images`, round 75)

Covers the DiskImageKit rows of the parity matrix (`docs/parity-matrix/data/diskimagekit.json`): creating, opening,
inspecting and converting RAW and ISO images, mounting them from Files, and detaching them in Disk Utility.

## Evidence used

- Reference: `~/Projects/macos-ref/extract/ref27-index.tsv` lists `System/Library/CoreServices/DiskImageMounter.app`.
  Its `Contents/Info.plist` and resources are **not extracted yet** (the lead extracts them under
  `~/Projects/macos-ref/extract/ref27/`). Behaviour here comes from the API names in the matrix and the existing code.
  Re-check `ref27/System/Library/CoreServices/DiskImageMounter.app` and any `hdiutil` man page before the round closes
  and adjust the option set (`-srcfolder`, `-size`, `-readonly`, `-readwrite`, `-format`, `-blocksize`, `-o`).
- Existing code before this round: `springboard/core/src/disk_image.rs` and a duplicate `springboard/ui/src/disk_image.rs`
  (both pure models), `springboard/apps/files/src/info.rs` (Get Info rows for images), `springboard/apps/diskutil-desktop`
  (disks through `bifrost-core` disk sessions and `udisksctl`), and `springboard/cmd/src/open.rs` (the control-socket client).
- Nothing has been compiled or run (round rule: code and commit only). Everything below needs the lead's build and a
  Midget check.

## Shared model

One model, `bifrost_core::disk_image` (`springboard/core/src/disk_image.rs`). The ui copy was deleted and
`springboard/ui/src/lib.rs` re-exports it (`pub use bifrost_core::disk_image;`), so `bifrost_app::controls::disk_image`
keeps working for Files.

- `BlockSize` (512, 4096), `ImageFormat` (RAW only), `parse_format` (case-insensitive, names any other format).
- `create_raw`, `open_raw`, `ImageProperties` (`byte_size`, `truncate`, `reblock`), with block-count checks.
- `parse_size` for `10m`, `2G` style sizes (binary suffixes, overflow refused).
- `inspect(head, len) -> Inspection` reads an ISO 9660 primary descriptor: volume name, declared blocks and block size,
  whether the file holds the whole volume. `verify(head, len)` turns an incomplete volume into `ImageError::Truncated`.
- `ImageError` with `type_name()` mapping to the official error types (`InvalidBlockCountError`,
  `UnsupportedFormatError`, `CorruptedImageError`).

## Rows

| Entry (matrix) | API | BifrOSt path | State |
|---|---|---|---|
| Create an empty disk image (RAW) | `DiskImage.init(creating:)`, `RAWCreationConfiguration` | `hdiutil create -size <n> -o <file>` (`springboard/cmd/src/hdiutil.rs`, `write_blank`): sparse file of the asked size, refuses an existing file | Code written. Not built, not run on Midget. |
| Create an ISO from a folder | (no matrix row) | `hdiutil create -srcfolder <dir> -volname <name> -o <file>`: runs `xorriso -as mkisofs` (or `genisoimage`, `mkisofs`) found on `PATH` | Code written. Needs an ISO writer installed on Midget (xorriso is a system package). |
| Open an existing disk image | `DiskImage.init(opening:)`, `OpenConfiguration` | `hdiutil attach [-readonly|-readwrite] <file>` → control line `image-attach ro|rw <path>` → `springboard/src/shell/disk_image.rs::attach`: privd `disk.loop_attach`, then `udisksctl mount -b` | Code written. Needs a Midget run with a real ISO and a session token. |
| Detach / release the image | (no direct row) | `hdiutil detach <device|folder>` → `image-detach` → `shell::disk_image::detach`: unmount, then privd `disk.loop_detach` | Code written. |
| Stacked disk images and layers | `StackedImage`, `appending(_:)`, `LayerType` | none | Still a gap: layered formats are not modelled (only RAW). |
| Block size and format values | `DiskImage.Format`, `DiskImage.BlockSize`, `LayerType` | `core/src/disk_image.rs` (`BlockSize`, `ImageFormat`, `parse_format`); `hdiutil -blocksize`, `-format raw` | Done in the model; `-format` accepts only raw. |
| Truncate or extend an image | `truncate`/`extend` | `core/src/disk_image.rs` `ImageProperties::truncate`; no CLI verb yet | Model only. The filesystem inside is not resized. |
| Inspect image properties | `DiskImage` properties | `core` `inspect`, `hdiutil info <file>`, Files Get Info rows (`apps/files/src/info.rs`, `image_rows`) | Done: volume name, declared size, completeness. |
| Verify an image | (framework overview) | `hdiutil verify <file>` (`verify_text`) | Done: fails a truncated volume, passes raw. |
| Convert an image | (framework overview) | `hdiutil convert <file> -format raw [-blocksize n] -o <file>` (`run`, `Convert`) | Copies the bytes after checking the whole-block layout. Not a format conversion (only raw exists). |
| Error types | `IncompatibleStackingError`, `InvalidBlockCountError`, `CorruptedImageError`, `UnsupportedFormatError` | `core/src/disk_image.rs` `ImageError::type_name` | Done for the three that exist. |
| Virtualization integration | storage from an image | `core/src/guest_vm.rs` still uses `create_raw` | Unchanged. |

## Files (double-click)

`apps/files/src/window.rs` `open`: an `.iso` or `.img` (`ops.rs` `is_disk_image`) runs `hdiutil attach <path>` on a
background queue, then `bifrost-apps --open <mount folder>`, so the volume opens in Files. Not in the Files menu or
the Open With panel yet.

## Disk Utility

`apps/diskutil-desktop` 0.2.0. `sysfs.rs` shows a loop device whose `loop/backing_file` is set as a disk image
(`Node.image_file`, type "Disk image" in the info pane). Other loop devices stay hidden. The toolbar Detach button and
the Image menu's Detach Disk Image run `hdiutil detach /dev/loopN`. Mount and Unmount already work on the image
through the existing session. Not in this round: attach, create, verify and convert dialogs in Disk Utility (they need
file pickers).

## privd

`springboard/src/privd/loopdev.rs`, methods `disk.loop_attach {path, read_only}` and `disk.loop_detach {device}`.
Attach needs an absolute `.iso` or `.img` path that is canonical (no link), a regular file owned by the installed
account, under the account's home. Detach only releases a loop device whose backing file is such an image under the
home folder. Both need the session token like every other non-unlock method.

## Midget checks still needed

1. `hdiutil create -size 10m -o ~/t.img`, then `hdiutil info` and `hdiutil verify` on it.
2. `hdiutil attach ~/some.iso` with the session running: a `/dev/loopN` line and a folder under `/run/media/<user>/`.
3. Double-click an `.iso` in Files: the volume opens. Then Disk Utility shows the loop device as a disk image, and
   Detach releases it (`losetup -a` shows nothing afterwards).
4. A symlink to an image, and an image outside the home folder, are refused by privd.
5. `hdiutil create -srcfolder` needs xorriso on Midget; without it the error says so.

## Known limits

- Only RAW images. ASIF and layered formats are Apple-only, status n/a.
- `image-attach` runs on the shell's control loop; a slow mount briefly blocks other control requests.
- `hdiutil` is the command name the brief asks for; the code and output are BifrOSt's own.
