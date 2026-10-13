# Changelog

## 0.2.0

- Erase and First Aid. Erase formats the selected disk (whole disk) or volume as FAT32 and First Aid checks a volume (Check Only, or Repair), each behind a confirmation sheet. A request the disks already refuse (a mounted volume, a partition where a disk is needed) shows its reason in the footer and opens no sheet.
- The erase and check requests are sent to the disk helper (`disk.*` in privd, `bifrost_core::disk_request`) on a thread with progress in the footer. The helper answers only an unlocked session; this app does not hold one yet, so it shows that refusal until the shell brokers the request.
- Mount, unmount and eject run their commands through the shared plan runner (`bifrost_core::disk_run`); the app's own copy is removed.

## 0.1.1

- Refusal sentences come from the disk session (`Refusal::sentence`), shared with the `diskutil` command line tool; the app's own copy is removed.

## 0.1.0

- New app: Disk Utility. A sidebar of internal and external disks with their partitions, and an info pane with
  capacity, used and available space, file system, volume name, mount point, device, type, writability and SMART status.
- Mount, unmount and eject through the disk session (claims and approval gates), run on a thread with progress and errors in the footer.
- No erase or partition operations in this version.
