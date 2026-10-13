# Parity: Disk Utility (slug `disk-utility`)

Round 75, wave 2. Owner: disk-utility slice. Reference: macOS 27 (26A434) `Disk Utility.app` and `usr/sbin/diskutil`.

## Evidence used (clean room)

- `~/Projects/macos-ref/extract/ref27-index.tsv`: the app bundle `System/Applications/Utilities/Disk Utility.app`
  (Info.plist and `Resources/*.lproj` present in the index; the lead had not yet extracted the app's nibs when this
  round ran, so no interface-builder layout was measured).
- `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/diskutil.8`: verbs `eraseDisk`, `eraseVolume`, `reformat`,
  `partitionDisk`, `verifyVolume`, `repairVolume`, `verifyDisk`, `repairDisk`, `mountDisk`, `unmountDisk`, `eject`.
  Behaviour used: an erase writes a new partition map with one volume (`eraseDisk`) or formats a partition in place
  (`eraseVolume`); a verify is read-only; a repair is a file system check that fixes errors; ownership of the disk is
  needed for each; a disk erase with `-noEFI` is not modelled (no EFI partition on Linux). The personalities offered
  are the file systems BifrOSt can make on Linux: ext4, exFAT and FAT32.
- Not used: `docs/parity-matrix/` rows (stale, not edited).

## Rows

| Matrix entry | Kind | BifrOSt path | Status |
|---|---|---|---|
| `Disk Utility` (apps/d-f.json, gap) | app | `springboard/apps/diskutil-desktop` | Partial. Sidebar, info pane, usage bar, mount, unmount and eject were already in place (0.1.0). Erase, first aid and partition planning now exist in core (below). No UI, no privd transport yet. |
| `diskutil` (cli/d-f.json, gap) | CLI | not built | Gap. The plans below are the logic a CLI would use; no `bin` added this round. |
| `DiskArbitration` (frameworks/d-f.json, gap) | framework | `springboard/core/src/disk_session.rs`, `disk_events.rs`, `disk_commands.rs` | Unchanged this round. |

## What is done (core, host-tested only, not built)

- `springboard/core/src/disk_format.rs` (new): plans for
  - `erase_volume`: an empty file system on an unmounted partition (`mkfs.ext4 -F -q [-L]`, `mkfs.exfat [-n]`,
    `mkfs.vfat -F 32 [-n]`);
  - `erase_disk`: `sgdisk --zap-all`, one GPT partition of the Linux (8300) or Microsoft basic data (0700) type,
    `partprobe`, `udevadm settle`, then the format on the partition (`sdb1`, or `nvme0n1p1` when the disk name ends in a
    digit). Refused when the disk or any of its partitions is mounted;
  - `check_volume` with `CheckMode::Verify` (`e2fsck -n`, `fsck.exfat -n`, `fsck.fat -n`) or `CheckMode::Repair`
    (`e2fsck -p`, `fsck.exfat -a`, `fsck.fat -a`). Refused when mounted; a type with no checker is refused.
  - Labels: at most 16 (ext4), 15 (exFAT) or 11 (FAT32) bytes; letters, digits, space, `-`, `_`; no leading or trailing
    space; empty means unlabelled.
- Every plan goes through the same gates as mount and eject: device name, claims, approvals (`Operation`), read-only,
  mounted. Commands are fixed programs with separate argument words; nothing runs a shell.
- `springboard/core/src/disk_events.rs`: `Operation` gains `Erase`, `Verify`, `Repair` (no other match on it exists).
- `springboard/core/src/disk_commands.rs`: `device_path`, `gate_claim`, `gate_approval` become `pub(crate)` so
  `disk_format` reuses them.
- `springboard/core/src/lib.rs`: `pub mod disk_format;`.

## What still needs work

- privd transport: the plans must run as root. `springboard/src/privd/proto.rs` is frozen by the lead and has no disk
  method, so nothing executes these plans yet. The lead needs to add a method (for example `disk.format` taking the
  command list or a fixed operation and parameters, with the same validation) and decide whether the run treats an
  fsck exit of 1 (errors corrected) as success.
- UI: an erase sheet (personality picker, label field, destructive confirmation in `DESTRUCTIVE`), a first-aid button
  with its progress and result, and a partition sheet. Not built: a footer error would be the only feedback today.
- Partition map choice: GPT only. Disk Utility offers APM and MBR schemes; not modelled.
- Host and Midget checks: the fsck exit codes and the exFAT checker's flags (`-a`, `-n`) need checking on a real
  system before the rows become `parity`.
- Erase of an optical disc, secure erase and zero-fill are not modelled.
