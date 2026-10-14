# Changelog

## 0.7.1 — 2026-10-14

- Run disk operations through the shared utility queue and wake the app loop when each result is ready, without polling.

## 0.7.0

- File > Open Disk Image attaches the chosen image read-only, as its menu name says. File > Open Disk Image for Writing attaches it with read and write access.
- Mount, Unmount and Eject in the File menu and the toolbar use the same rules, so a command is enabled in one exactly when it is enabled in the other.
- Software RAID arrays (md devices) are listed in the sidebar, so an array the RAID Assistant made can be selected. The RAID Assistant never offers an array as a member.

## 0.6.0

- Form sheets with text fields, choices and lists, in the sheet style of the other dialogs.
- Disk > Rename changes the name of an unmounted volume (ext4, exFAT, FAT32, NTFS and btrfs).
- Disk > Erase asks for a name, a format (ext4, exFAT, FAT32, NTFS or btrfs) and, for a whole disk, a partition scheme (GUID Partition Map or Master Boot Record) before the confirmation. NTFS and btrfs join the Format menu items and the first-aid checks.
- Disk > Restore copies a volume, or an attached disk image, onto the selected volume. The source must be unmounted and fit.
- Disk > RAID Assistant creates a software RAID 0, 1, 5, 6 or 10 array from the unmounted disks and volumes you tick.
- File > New Image > Blank Image makes a sparse raw image of a chosen size, optionally with an empty file system and volume name, in a place chosen in the save panel. File > New Image > Image from Folder builds an ISO image of a folder.
- File > Open Disk Image attaches an .iso or .img file through the open panel.
- Image > Convert Image writes an image as raw, QCOW2, VMDK, VHDX or VDI. Image > Resize Image grows an image, or shrinks an ext image after shrinking the file system inside it.
- Get Info adds the UUIDs, block sizes, transport, serial number and the other facts `lsblk` knows, and is wide enough not to cut lines short.
- The confirmation sheets wrap their text instead of cutting it off, and Escape cancels one.
- The app now asks for the documents permission, for the open and save panels.

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
