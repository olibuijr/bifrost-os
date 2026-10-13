# Changelog

## 0.1.1

- Refusal sentences come from the disk session (`Refusal::sentence`), shared with the `diskutil` command line tool; the app's own copy is removed.

## 0.1.0

- New app: Disk Utility. A sidebar of internal and external disks with their partitions, and an info pane with
  capacity, used and available space, file system, volume name, mount point, device, type, writability and SMART status.
- Mount, unmount and eject through the disk session (claims and approval gates), run on a thread with progress and errors in the footer.
- No erase or partition operations in this version.
