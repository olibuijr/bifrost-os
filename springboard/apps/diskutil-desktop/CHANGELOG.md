# Changelog

## 0.2.0

- Disk images: a loop device that holds an .iso or .img shows in the sidebar as a disk image, with its image file in the info pane. Detach (toolbar, or Image > Detach Disk Image) unmounts it and releases the loop device through `hdiutil detach`.

## 0.1.0

- New app: Disk Utility. A sidebar of internal and external disks with their partitions, and an info pane with
  capacity, used and available space, file system, volume name, mount point, device, type, writability and SMART status.
- Mount, unmount and eject through the disk session (claims and approval gates), run on a thread with progress and errors in the footer.
- No erase or partition operations in this version.
