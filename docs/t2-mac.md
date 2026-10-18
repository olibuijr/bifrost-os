# T2 MacBook Pro installer media

The x86_64 installer has an opt-in T2 build. It uses the pinned `linux-lts-t2` package for the live system and includes
the in-kernel T2BCE modules, `applespi`, and `hid-appletb-kbd` in the initramfs. The EFI command line includes
`efi=noruntime intel_iommu=on iommu=pt pm_async=off`.

Build with:

```sh
BIFROST_ISO_T2=1 BIFROST_ISO_OUTPUT_DIR=/path/to/output tools/build-iso.sh --release --profile fast
```

The ordinary ISO build remains the default. The T2 build pins and verifies the kernel package, adds it to the live
package set, and embeds it in the offline installer repository. During installation, the installer detects an Apple
MacBook Pro through DMI, selects `linux-lts-t2`, and writes the T2 kernel command line and initramfs preset to the new
system. The physical Mac boot and built-in input devices still need validation on the target model.

On the Mac, select the USB's EFI boot entry from Startup Manager. If external media is blocked, allow external boot in
Startup Security Utility. T2 boot setup notes are maintained by the [T2 Linux Arch installation guide](https://wiki.t2linux.org/distributions/arch/installation/).
