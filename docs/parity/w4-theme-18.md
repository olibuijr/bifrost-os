# Wave 4, theme-18: disk description and lookup, claims and volume commands, ICC profile inspection

Status of this round: pure Rust models with host unit tests, written without a build (the wave rule for this round).
Nothing is wired into the shell, the apps or the installer, and nothing has run on the VM or Midget yet. Rows move to
`partial`, not `parity`.

## What was built

- **`springboard/core/src/disk_events.rs` (extended)**: `description` lists a device's properties as ordered
  name/value pairs (`mount_point` is left out while unmounted); `lookup_bsd_name` finds a device by its name;
  `lookup_volume_path` returns the mounted device whose mount point is the deepest directory containing a path
  (whole-component match, so `/media/stick` does not contain `/media/sticky`; a root mount contains every absolute path).
- **`springboard/core/src/disk_claims.rs` (new)**: an exclusive claim registry keyed by device name. `claim` and
  `unclaim` refuse a second owner and report the holder, a repeated claim by the holder succeeds, and `check` gives the
  rule the commands use. It takes no lock on the hardware.
- **`springboard/core/src/disk_commands.rs` (new)**: plans mount, unmount, eject and label change as `Command` lists
  (`udisksctl mount|unmount|power-off`, `fatlabel`, `e2label`). Each request is checked in order: device name (plain
  alphanumeric, so no path escapes `/dev`), claims, the existing `Approvals` chain for `Mount`/`Unmount`/`Eject`, and
  device state (already mounted, not mounted, read-only). Eject of a mounted volume plans an unmount first. Labels are
  limited to 11 bytes (FAT) or 16 bytes (ext), printable ASCII, no slash, no space. Nothing is executed.
- **`springboard/render/src/icc.rs` (new)**: `Profile::parse` checks the 128-byte header, the `acsp` magic and the tag
  table, and refuses a tag whose data runs past the declared size. `Profile::description` reads the `desc` (version 2)
  or `mluc` (version 4, UTF-16BE) text. `Devices` records the default and custom profile of each display, printer,
  scanner and camera; `display_profile` falls back to the sRGB profile for an unregistered display; `iterate` lists
  devices by class and identifier.

Tests (written by hand, not run): 6 new in `disk_events` alongside its 7 existing ones, 6 in `disk_claims`, 11 in
`disk_commands`, 12 in `icc`. The ICC tests build profiles byte by byte in the test module.

## Official API names mirrored

- Disk Arbitration: `DADiskCopyDescription`, `DADiskCreateFromBSDName`, `DADiskCreateFromVolumePath`,
  `DADiskGetBSDName`, `DADiskClaim`, `DADiskUnclaim`, `DADiskIsClaimed`, `DADiskMount`, `DADiskMountWithArguments`,
  `DADiskUnmount`, `DADiskEject`, `DADiskRename` (DocC index: `diskarbitration`, symbol `dadisk`).
- ColorSync: the `Color devices` topic ("Manage the color profiles assigned to displays, printers, scanners, and
  cameras") and the `Color profiles` topic ("Work with the ICC profiles that describe device and working color spaces"),
  as listed on the rendered framework page. No ColorSync symbol is mirrored by name; the Rust API is new.
- Image I/O: the colour-profile topic only (`Color management` keys are not modelled; no gain map or HDR support).

## REA evidence (rendered official pages, `tools/parity/rea-page.sh`)

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/diskarbitration | ev_23f638db4a460a01398175e41eb2803e547caac9654d4fb856213954e99910c1 |
| https://developer.apple.com/documentation/diskarbitration/dadisk | ev_a1aa42a0daca39d4f3e58248270652581f159eaad03714720ffa0d8968986c88 |
| https://developer.apple.com/documentation/colorsync | ev_964d39023df4ecce4433bba12af532398d3da272281c2c1972eb035c2376c8f8 |
| https://developer.apple.com/documentation/imageio | ev_c399bf3c8c60909a1c96e42ab015667f31a745806bab4d58b77613d1b5d9f527 |

The DocC index (`docc.py diskarbitration --depth 2`) lists `DADiskClaim(_:_:_:_:_:_:)`, `DADiskMount(_:_:_:_:_:)`,
`DADiskMountWithArguments`, `DADiskUnmount`, `DADiskEject`, `DADiskRename` and `DADiskUnclaim`, each with its
callback and options type.

## Matrix rows closed (new status partial)

| Framework slug | Feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| disk-arbitration | Disk description and lookup (DADisk, DADiskCreateFromBSDName, DADiskCreateFromVolumePath, DADiskGetBSDName, DADiskCopyDescription) | springboard/core/src/disk_events.rs |
| disk-arbitration | Disk claims (DADiskClaim, DADiskUnclaim, DADiskIsClaimed) | springboard/core/src/disk_claims.rs |
| disk-arbitration | Mount, unmount, eject and rename commands (DADiskMount, DADiskUnmount, DADiskEject, DADiskRename) | springboard/core/src/disk_commands.rs |
| image-i-o | Colour profiles and HDR gain maps | springboard/render/src/icc.rs |
| colorsync | Device and display profiles | springboard/render/src/icc.rs |

## Not closed in this round

- **Image I/O, Format coverage (GIF, HEIC, WebP, TIFF, TGA, DNG, RAW)**: decoders for these formats are not written.
  Out of scope for pure models here.
- **DiskImageKit, Inspect image properties, Error types**: not built. The error enum in `disk_commands` is a refusal
  type for the disk requests, not the image framework's error set.
- **ColorSync, Color profiles (ICC load, create, inspect, embed)**: `icc.rs` inspects the header and tags only. It does
  not create or embed profiles, and it is not yet used by the PDF reader or Photos.
- **Disk Arbitration callbacks and approvals** (wave 2 rows, theme-20): unchanged. `disk_commands` consumes the existing
  `Approvals` chain; it does not add callbacks.

## Limits and what would prove them

- The claim registry is in-process only. Nothing stops another process from touching the device; a real claim needs a
  system service that the helper commands go through.
- The command plans assume `udisksctl`, `fatlabel` and `e2label` exist on the target. These are not checked on Midget.
- `lookup_volume_path` uses the mount points the caller passes in. It does not read `/proc/mounts`.
- The ICC parser checks structure and bounds. It does not validate the colour data or the PCS values.
- Verification still to do: host tests (`cargo test -p bifrost-core disk` and `-p bifrost-render icc`), then the VM and
  Midget, which is the final testbed under the parity rules.
