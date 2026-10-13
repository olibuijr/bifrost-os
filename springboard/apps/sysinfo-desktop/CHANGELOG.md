# Changelog

## 0.3.0

- The size, clock and USB speed labels come from `bifrost_core::sysinfo`, shared with the `system_profiler` command line tool.

## 0.2.0

- Network lists each interface's global IPv6 addresses (from `/proc/net/if_inet6`) and marks the interface that carries the default route (from `/proc/net/route`).

## 0.1.0

- New: System Information, with the hardware tree (processor, memory, graphics, PCI, USB, displays, network, power, storage) and the software section (operating system and installed apps).
