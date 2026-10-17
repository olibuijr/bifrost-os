# Changelog

## 0.9.0

- Software > Installations lists the installed packages read from the pacman local database (`/var/lib/pacman/local`, `desc` files; no pacman run), newest install first, then the package changes from the log.
- PCI Devices take vendor and device names from `/usr/share/hwdata/pci.ids` (or `/usr/share/misc/pci.ids`) when present; otherwise the built-in vendor names and the hexadecimal ID are shown.

## 0.8.2 — 2026-10-14

- Derive window geometry from the configured content size rather than a fixed-resolution fallback.

## 0.8.1

- Routed Copy and Select All through the shell Edit menu; app search and speech actions have separate menus. Added the compliance test harness.

## 0.8.0

- Help > System Information Help shows a short help card in the window: what each part of the window does and which keys and menu items reach it. Choosing a category or starting a find brings the facts back. Help > System Information Online Help opens the web page, as before.

## 0.7.0

- Edit > Speak Serial Number reads the serial through the system speech engine (`espeak-ng`, else `espeak`), whether or not the screen reader is on. With no engine installed the announcement shows as the banner, as before.
- The manifest declares the JSON type (`public.json`, rank none), so Files lists System Information under Open With for a saved report. Files still does not pick it by itself.

## 0.6.0

- Edit > Use Selection for Find (Cmd+E) opens the find bar with the picked value or item; Edit > Jump to Selection (Cmd+J) scrolls the picked row or item into view.
- Item lists have a Name and Details header. A click sorts by the column and a second click reverses it; the USB tree keeps its bus order. The picked item stays picked through a sort and through a refresh.
- Processor lists the clock of each processor and, on hybrid processors, the performance and efficiency core counts.
- Displays show each display's refresh rate and scale, from the display service.
- Hardware adds Card Reader, Controller, SAS, Fibre Channel, Disc Burning and Diagnostics (crash reports and core dumps).
- Software adds Installations (the package log, newest first) and Printer Software (the installed printer drivers).

## 0.5.0

- New categories: Audio, Bluetooth, Camera, Printers, Thunderbolt, NVMe and SATA under Hardware; a Network section with Wi-Fi, Ethernet, WWAN, Firewall and Volumes; Fonts and Logs under Software.
- Categories that list devices, disks or apps show an item list above a detail card for the picked item. Up and Down move through the list when it has the focus; Tab cycles the sidebar, the list and the card.
- Processor adds physical cores, cache sizes and the clock range. Memory adds available memory, swap and one row per memory slot with type, speed and maker when the firmware table can be read. Graphics adds video memory.
- USB is a tree (bus, hub, device) with each device's maximum power. Storage lists each partition with its file system, mount point and free space.
- Network interfaces show IPv4 addresses, the gateway, the DNS servers, the link speed and the Wi-Fi signal. Operating System adds the build and the secure boot state. Installed Apps show the app's name, kind and install date.
- File > Save Report As Text, Rich Text or JSON; File > Open Report reads a JSON report in place of this computer's facts, with Open Recent and Show This Computer. The manifest now asks for the documents permission the save and open panels need.
- Edit > Speak Serial Number announces the serial digit by digit. Window menu: Show Overview, Previous and Next Category.
- The search field in the toolbar filters the rows as you type, with Find Next and Previous in the Edit menu.
- The sidebar sections fold and unfold, the choice is remembered, and the sidebar scrolls.
- The facts are read on a worker thread behind a "Preparing system profile" sheet with Cancel.

## 0.4.0

- Menus: File > Save Report As (text), Edit > Copy, Select All, Find, Find Next and Previous, View > Refresh Information (Cmd+R) and Show Less Information, Help.
- Detail rows can be picked (click or Select All) and copied as `key: value` lines.
- Find filters the rows of a category and jumps to the next category with a match.
- Up and Down move the sidebar selection; Tab switches between the sidebar and the card.
- Operating System adds computer name, time since boot, boot volume and boot mode; new Extensions (kernel modules), Startup Items and Language & Region categories.

## 0.3.0

- The size, clock and USB speed labels come from `bifrost_core::sysinfo`, shared with the `system_profiler` command line tool.

## 0.2.0

- Network lists each interface's global IPv6 addresses (from `/proc/net/if_inet6`) and marks the interface that carries the default route (from `/proc/net/route`).

## 0.1.0

- New: System Information, with the hardware tree (processor, memory, graphics, PCI, USB, displays, network, power, storage) and the software section (operating system and installed apps).
