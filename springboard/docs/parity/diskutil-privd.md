# Disk Utility: erase, partition and first aid through privd (diskutil-privd)

Round 75, slice `diskutil-privd`. Branch `worktree-wf_2c5f5341-c36-5`, commits prefixed `parity(diskutil-privd):`.

## Evidence used

- `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/diskutil.8`, VERBS section: `verifyVolume`, `repairVolume`
  (fsck of an unmounted volume, Ownership required), `eraseDisk format name device` (whole disk, one new volume),
  `eraseVolume format name device` (partition keeps its place, data lost), `partitionDisk`, `verifyDisk` and `repairDisk`
  (partition map checks; not implemented here). Running as root is required for the write verbs.
- `ref27/System/Applications/Utilities/Disk Utility.app` exists in the extract; its Info.plist and nibs were not measured
  in this round (no REA evidence ids are recorded; the man page was enough for the verb set).
- Existing code: `springboard/core/src/disk_format.rs` (wave 2 plans), `disk_commands`, `disk_session`, `disk_events`.

## Rows

| Entry (matrix) | BifrOSt path | Status here |
|---|---|---|
| Disk Utility (`os/apps/d-f.json`, gap) | `apps/diskutil-desktop` (toolbar Erase and First Aid, confirmation sheet, footer progress) | partial: erase and first aid built; no partition in the app; not yet reachable from the app process (see below) |
| diskutil (`os/cli/d-f.json`, gap) | `cmd/src/bin/diskutil.rs`, `cmd/src/diskutil.rs` (`eraseDisk`, `eraseVolume`, `partitionDisk`, `verifyVolume`, `repairVolume`) | partial: runs as root (sudo), through the helper's plans |
| disk-arbitration (`data/disk-arbitration.json`) | not closed by this slice: no appearance or approval events were added | unchanged |

## What is done

- `core/src/disk_format.rs`: `partition_disk` (one GPT partition per spec, `sgdisk --zap-all` then one `sgdisk` create, `partprobe`, `udevadm settle`, a format per partition). Last partition may take the rest of the disk. Up to 16.
- `core/src/disk_request.rs`: the request contract shared by privd, the CLI and the app: method names (`disk.erase_disk`, `disk.erase_volume`, `disk.partition_disk`, `disk.verify_volume`, `disk.repair_volume`), wire parameters, confirmation sentence, progress word, and `plan` against a snapshot.
- `core/src/disk_scan.rs`: the `/sys/block` and `/proc/mounts` reads, moved out of the CLI.
- `core/src/disk_run.rs`: the one plan runner (a repairing checker's exit 1 counts as fixed). Replaces the CLI's and the app's copies.
- `core/src/privd_wire.rs`: the one socket client for the helper. `springboard/src/privd_client.rs` uses it.
- `springboard/src/privd/disk.rs`: the five `disk.*` methods, dispatched from `privd/methods.rs`. Params are validated, the
  disks are re-read here, `blkid` gives the type to check, the plan runs as root. They need the unlocked token like the other AUTH methods.
- `cmd`: `diskutil eraseDisk|eraseVolume|partitionDisk|verifyVolume|repairVolume`. A user (not root) gets "needs root. Run it with sudo."
- `apps/diskutil-desktop` 0.2.0: Erase (FAT32, whole disk or volume) and First Aid (Check Only, Repair) open an alert sheet
  (`bifrost_app::widgets::sheet`). A request the disks already refuse shows the reason in the footer and no sheet opens.
  Progress is the footer line "Erasing sdb..." while the request runs on a thread.

## Migrations and deletions

- `cmd/src/diskutil.rs`: `Entry`, `parse_mounts`, `unescape`, `scan` and their test moved to `core/src/disk_scan.rs`; `run` replaced by `disk_run::run_plan`.
- `apps/diskutil-desktop/src/actions.rs`: local `run` replaced by `disk_run::run_plan`.
- Refusal gains `NotWhole` and `NotPartition` (sentences in `disk_commands.rs`).

## What still needs checking on Midget (not built, not run here)

- The helper refuses the app: the app holds no session token and its binary is not in privd's admitted executables
  (`/usr/bin/springboard`, `/usr/bin/bifrost-mcp`). The app's erase and first aid therefore show "The disk helper needs an
  unlocked BifrOSt session, and this app does not have one." until the shell brokers the request or privd admits the app.
  This is a decision for the lead (a new peer rule or a shell-side proxy); no privilege was widened in this slice.
- `sgdisk`, `partprobe`, `udevadm`, `mkfs.*`, `e2fsck`, `fsck.fat`, `fsck.exfat` and `blkid` must exist on the installed system.
- Erase and partition on a real stick (Midget only; never Titan).
- Progress is indeterminate: the helper answers once the plan has run; there is no per-step event.
- `verifyDisk` and `repairDisk` (partition map checks) are not implemented.
- Claims (`disk_claims`) do not cross processes: the helper plans with an empty claim set.
