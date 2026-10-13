# Changelog

## 0.6.0

- Sheets can take text: a name or size field with the keyboard (Tab between fields, Return confirms, Escape cancels).
- Erase asks for the volume name; the Disk menu adds NTFS and Btrfs to the formats.
- Disk > Rename renames an unmounted volume (ext4, exFAT, FAT32, NTFS, Btrfs) through the disk helper.
- Disk > Partition (and a toolbar button) takes the size of each partition, such as `64 GB, 25%, rest`.
- Disk > Restore copies an unmounted volume or disk onto another of the same kind that is at least as large.
- File > New Blank Image creates and formats an image in Documents and opens it; File > Open Disk Image attaches an image file.
- Images > Convert to qcow2 and Convert to Raw write a converted copy of the selected image next to it.
- Get Info lists the UUID, partition map, serial number and block size.

## 0.5.0

- File > Show in Files (cmd+shift+R) opens the selected volume's mount point in the file manager.

## 0.4.0

- Menus: File (Get Info, Mount, Unmount, Eject with shortcuts), View (Show Only Volumes, Show All Devices) and Disk (Erase, Run First Aid, the erase format, Partition into 2 to 4 equal parts) join the Image menu.
- Show Only Volumes lists the volumes without their disks; Show All Devices is the previous disk tree.
- Get Info opens a sheet with every fact about the disk or volume, including model, parent disk and image file.
- Erase writes the format chosen in the Disk menu (ext4, exFAT or FAT32) and keeps a volume name the format accepts.
- Partition replaces a disk's partitions with equal parts in the chosen format, behind a confirmation sheet, through the disk helper.

## 0.3.0

- Erase and First Aid. Erase formats the selected disk (whole disk) or volume as FAT32 and First Aid checks a volume (Check Only, or Repair), each behind a confirmation sheet. A request the disks already refuse (a mounted volume, a partition where a disk is needed) shows its reason in the footer and opens no sheet.
- The erase and check requests are sent to the disk helper (`disk.*` in privd, `bifrost_core::disk_request`) on a thread with progress in the footer. The helper answers only an unlocked session; this app does not hold one yet, so it shows that refusal until the shell brokers the request.
- Mount, unmount and eject run their commands through the shared plan runner (`bifrost_core::disk_run`); the app's own copy is removed.

## 0.2.0

- Disk images: a loop device that holds an .iso or .img shows in the sidebar as a disk image, with its image file in the info pane. Detach (toolbar, or Image > Detach Disk Image) unmounts it and releases the loop device through `hdiutil detach`.
- Refusal sentences come from the disk session (`Refusal::sentence`), shared with the `diskutil` command line tool; the app's own copy is removed.

## 0.1.0

- New app: Disk Utility. A sidebar of internal and external disks with their partitions, and an info pane with
  capacity, used and available space, file system, volume name, mount point, device, type, writability and SMART status.
- Mount, unmount and eject through the disk session (claims and approval gates), run on a thread with progress and errors in the footer.
- No erase or partition operations in this version.
