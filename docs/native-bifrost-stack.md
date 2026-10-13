# Native BifrOSt stack

This is the removal ledger for the PC boot path. “Native” means a BifrOSt-owned
Rust component owns the responsibility before the corresponding external
component is removed.

## Ownership ledger

| Stage | Current owner | Native BifrOSt owner | Removal gate |
|---|---|---|---|
| Firmware POST and Lenovo logo | Lenovo UEFI | Firmware | Not replaceable by an OS |
| EFI handoff and USB discovery | Rust `springboard/bootweave/efi` | Rust EFI boot app | EFI fast/release build and USB boot |
| Kernel and hardware drivers | Linux kernel and firmware | BifrOSt policy around Linux | Hardware bring-up, suspend, USB, graphics |
| Early splash and handoff | Rust EFI frame plus Bootweave | Rust Bootweave supervisor | No black/text transition on OVMF and hardware |
| Live installer display | Wayland session (`bifrost-wm`) | `bifrost-installer` direct DRM/KMS | Installer owns the card and completes install |
| Installed desktop display | `bifrost-wm` (native compositor) | `bifrost-wm` | Shell, apps, input, hotplug and recovery |
| Service startup | systemd units and scripts | Rust BifrOSt supervisor | Boot, restart, logs, recovery and updates |

## Removal rule

Do not remove Linux, motherboard firmware, GPU firmware, or USB controller
firmware: these are hardware/runtime prerequisites, not BifrOSt desktop glue.
Do not remove Wayland session glue or systemd until the native
replacement owns the same stage and has passed the removal gate. Bootweave
now owns the Plymouth-sized lifecycle boundary; labwc is gone: `bifrost-wm` is the only window server.

The first native slice is already in place: the Rust EFI image is the shared
ISO/installed recovery artifact, and the live installer starts directly on
DRM/KMS. Bootweave is the first early-userspace handoff slice: it retains the
last EFI frame while starting and supervising the installer or installed
compositor, and records compositor/first-frame timing. It is not a second
splash compositor. labwc and its session-only packages are removed.

## Acceptance storyboard

1. Lenovo firmware screen.
2. BifrOSt EFI logo and progress on internal disk or USB.
3. One retained BifrOSt frame across kernel and early-userspace handoff.
4. Native installer or installed desktop, with no console or generic display.
5. USB input/display hotplug, reboot, rollback and recovery use the same
   BifrOSt visual language.
