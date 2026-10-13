# Changelog

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
