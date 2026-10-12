# The whole reference OS in the parity matrix

Owner request 2026-10-12: register the reference OS in its entirety in the parity matrix and the published database,
not only its public frameworks, and use reverse engineering (the `reverse-engineer-anything` skill, REA) to bring each
part to BifrOSt: a de-Appled macOS-style OS for x86 PCs.

## Source

- macOS 27 Golden Gate 27.0.1, build **26A434** (product 142-27367), from Apple's public software update catalog
  (`swscan.apple.com`, publicrelease) with `corpnewt/gibMacOS`.
- `~/Projects/macos-ref/downloads/InstallAssistant.pkg` (18.39 GB). Tools: `7zip`, `blacktop/ipsw` 3.1.736 (checksum
  verified, `~/Projects/macos-ref/tools/ipsw/x/ipsw`), `apfs-fuse`, REA.
- Nothing from the image is committed to this repository. `~/Projects/macos-ref/` is outside it.

## Clean-room rule

The inventory records names, identifiers, versions, interfaces, structure and observed behaviour, with REA evidence.
BifrOSt reimplements behaviour from that record under neutral names (`bifrost pc names`). It never copies Apple code,
compiled shaders, artwork, icons, fonts, sounds, wallpapers or strings into BifrOSt, even as reference files in the
build. Shader inventories list libraries and entry-point names; BifrOSt writes its own shaders for the effects.

## Extraction

1. `7z x InstallAssistant.pkg SharedSupport.dmg`, then `7z x SharedSupport.dmg`: the
   `com_apple_MobileAsset_MacSoftwareUpdate/<hash>.zip` OS asset.
2. `ipsw ota ls` and `ipsw ota extract` on the asset: the system volume payloads, the cryptexes (the OS and app
   cryptexes hold the dyld shared cache and Safari-class apps), and the dyld shared cache.
3. Walk the extracted tree into the inventory (`tools/parity/os_inventory.py`, to be written): Info.plists, Mach-O
   load commands and exports, metallib headers and function lists, launchd plists, preference panes and Settings
   extensions.

## What gets registered

A new layer of the matrix, `docs/parity-matrix/os/<category>.json`, one entry per component:

| Category | Contents |
|---|---|
| `apps` | `/System/Applications`, `/Applications` (bundled), `/System/Library/CoreServices/*.app` |
| `system-ui` | Dock, Finder, Control Center, Notification Center, SystemUIServer, Spotlight, login window, WindowServer-side UI |
| `settings` | System Settings panes and extensions, the legacy preference panes |
| `daemons` | `/System/Library/LaunchDaemons` and `LaunchAgents`: each daemon with its role |
| `frameworks` | Public frameworks (already in `data/`) and private frameworks, from the dyld shared cache |
| `drivers` | Kernel extensions and DriverKit extensions, by device class |
| `shaders` | Every `.metallib` (and Metal libraries embedded in binaries): owner framework, entry points, purpose |
| `cli` | `/usr/bin`, `/usr/sbin`, `/usr/libexec` tools |
| `formats` | Uniform type declarations, document types, URL schemes |
| `assets` | Fonts, sounds, wallpapers, cursors, icons: inventory only, never copied |

Each entry: path, kind, identifier, version, description, owner (framework or app), public documentation link if any,
BifrOSt counterpart (path), status (the same `parity`, `partial`, `os`, `gap`, `n/a`), REA evidence ids. Coverage
counts the new layer separately from the public-framework matrix, so the existing 80 % deploy gate is unchanged.

`bifrost pc parity` gains the layer (`status --os`, `show os/<category>`), `db-export` writes an `os` collection, and
the published database page gets an OS tab.

## Status (2026-10-12)

- The installer download is in progress; extraction and the walker follow.
- From public documentation, before the dig: Core Image (10 filter-family rows), SwiftUI (shader effects, Liquid
  Glass, materials), AppKit (the visual effect material catalogue, glass effect views) and Core Animation (layer
  filters) rows were added to `data/`. Their per-shader detail comes from the `shaders` category.
