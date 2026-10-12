# Wave 3, theme 29: running Linux guests from an app

Theme: "Running Linux guests from an app" (Virtualization, DiskImageKit). Status of this work: partial.

## What was built

Two pure modules in `springboard/core` (std only, no unsafe, no hardware, no processes started):

- `springboard/core/src/guest_vm.rs`: the guest configuration check (`GuestConfig::validate`), the runtime state
  machine (`State`, `Machine`, `allowed`, `request_stop`), the device list (`Device`: entropy, socket, directory
  share, storage), the caching and sync enums (`CachingMode`, `SyncMode`), and the error types (`VmError`,
  `ConfigError`). A launcher outside this module is still missing; nothing boots a guest yet.
- `springboard/core/src/disk_image.rs`: RAW image geometry (`create_raw`, `open_raw`, `parse_format`), the block size
  and format values (`BlockSize`, `ImageFormat`), image properties (`ImageProperties`), truncate and extend
  (`ImageProperties::truncate`), and `ImageError`. No file is created, opened or resized; the caller does the I/O.

The state names mirror `VZVirtualMachine.State` (stopped, starting, running, pausing, paused, resuming, stopping,
saving, restoring, error). The rule for which step follows which is this project's reading of the documented
start, pause, resume, stop and request-stop behaviour. Apple's save and restore states are modelled as edges
(paused to saving, stopped to restoring) and are not yet exposed to any launcher.

Limits marked "BifrOSt" in the code (minimum memory 64 MiB, 64-byte share tag) are project choices and are not
documented framework values.

## Official API names mirrored

- Virtualization: `VZVirtualMachine`, `VZVirtualMachine.State` (and its `can*` properties), `start`, `pause`,
  `resume`, `requestStop`, `stop`, `VZVirtualMachineConfiguration` (CPU, memory, devices), the caching and disk
  synchronization modes, the entropy and socket devices, and VirtioFS directory sharing.
- DiskImageKit: `DiskImage` (create, open, properties), `DiskImage.BlockSize`, `DiskImage.Format` (RAW only),
  truncate and extend, and the storage attachment passed to the virtualization framework.

Not built, and deliberately left as gaps: the VM object and hypervisor binding, the display view, USB, custom
virtio drivers, stacked and ASIF images, and the image error cases that need file I/O.

## REA evidence

Official rendered pages captured with `tools/parity/rea-page.sh`:

- `ev_e507e82dc608246dedad90c9e9e8a581cdb9e16b39b42ee3e89e1acef8459642`: VZVirtualMachine
  (https://developer.apple.com/documentation/virtualization/vzvirtualmachine)
- `ev_8c7ff35e944b6fa7141785754649d23464c8928ca40b1b78e0547f5a5eb0b539`: VZVirtualMachine.State
  (https://developer.apple.com/documentation/virtualization/vzvirtualmachine/state)
- `ev_9e9a636398ca0b74df3013e6dcb77afd4bcb74b48c3ef19e86ba8a1ed920f569`: DiskImageKit
  (https://developer.apple.com/documentation/diskimagekit)

The DocC JSON endpoints (`tools/parity/docc.py`) returned 404 for these paths during this run, so the REA rendered
pages are the evidence used.

## Matrix rows closed (status partial)

| Framework slug | Feature (as written in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| virtualization | Running Linux guests from an app | partial | springboard/core/src/guest_vm.rs (`GuestConfig::validate`) |
| virtualization | VM runtime (start, pause, resume, state) | partial | springboard/core/src/guest_vm.rs (`Machine`, `allowed`) |
| virtualization | Errors and enumerations | partial | springboard/core/src/guest_vm.rs (`VmError`, `ConfigError`, `CachingMode`, `SyncMode`) |
| virtualization | Sockets | partial | springboard/core/src/guest_vm.rs (`Device::Socket`, one per guest) |
| virtualization | Shared directories | partial | springboard/core/src/guest_vm.rs (`DirectoryShare`, tag checks) |
| virtualization | Randomization | partial | springboard/core/src/guest_vm.rs (`Device::Entropy`, one per guest) |
| diskimagekit | Virtualization integration | partial | springboard/core/src/guest_vm.rs (`StorageAttachment::from_image`), springboard/core/src/disk_image.rs |
| diskimagekit | Truncate or extend an image | partial | springboard/core/src/disk_image.rs (`ImageProperties::truncate`) |

Not closed from the theme list: USB devices, Displaying a guest in a window, and Custom Virtio drivers, because no
honest pure model exists for them.

## Verification

- No builds were run, per the wave rules. The tests were written by hand from the rules the code implements
  (`disk_image`: 6 tests; `guest_vm`: 9 tests). They have not been executed.
- `python3 tools/check-names.py` reports no hits for the two new files.
- The akurai `code_search` call timed out during this run, so collision checks used plain grep over
  `springboard/core/src` (no existing `guest`, `vm` or `disk_image` module). Recorded as a known index problem.
