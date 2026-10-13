# Parity: system information command line (cli-system-info)

Slice: `sw_vers`, `system_profiler`, `networksetup` and `scutil`, in the `springboard/cmd` crate (package `bifrost-cmd`),
one binary per tool in `cmd/src/bin/`, logic in `cmd/src/<module>.rs` with unit tests. Built on the Wi-Fi service
(`is.olibuijr.net`, through `bifrost-proto`) and the system readers in `bifrost-core`.

## Evidence used

Clean room. Names, options and output shapes come from the shipped reference files, read as text only; nothing was
copied into the repo.

- `~/Projects/macos-ref/extract/ref27/usr/share/man/man1/sw_vers.1`: options and output lines.
- `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/system_profiler.8`: `-listDataTypes`, `-json`, `-detailLevel`
  (mini, basic, full), `-timeout`, `-usage`.
- `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/scutil.8`: `--get`, `--set`, `--dns`, `--proxy` and the
  preference names `ComputerName`, `LocalHostName`, `HostName`.
- `~/Projects/macos-ref/extract/ref27/usr/share/man/man8/networksetup.8`: the option list (services, hardware ports,
  Wi-Fi power, network and preferred networks, DNS and search domains, proxies, bypass list).
- Index: `usr/bin/sw_vers`, `usr/sbin/system_profiler`, `usr/sbin/scutil`, `usr/sbin/networksetup` in `ref27-index.tsv`.
- No REA evidence ids: the binaries were not inspected with REA in this round. The tools' data types and option names
  were taken from the man pages only.

## Rows

| Matrix entry | BifrOSt path | Done | Still needs |
|---|---|---|---|
| `sw_vers` (`path:/usr/bin/sw_vers`) | `cmd/src/sw_vers.rs`, `cmd/src/bin/sw_vers.rs` | Default listing, `-productName`, `-productVersion`, `-productVersionExtra`, `-buildVersion`, lower-case long options. Read from os-release. The product name is `BifrOSt`. | Midget check of the output against the running system. |
| `system_profiler` (`path:/usr/sbin/system_profiler`) | `cmd/src/profile.rs`, `cmd/src/bin/system_profiler.rs` | `-listDataTypes`; the seven data types (`SPHardwareDataType`, `SPSoftwareDataType`, `SPNetworkDataType`, `SPStorageDataType`, `SPDisplaysDataType`, `SPAudioDataType`, `SPUSBDataType`), each by id or heading; `-json`; `-detailLevel mini|basic|full`; `-timeout` accepted (no effect); `-xml` refused. Audio comes from `/proc/asound/cards`. | Gap: no XML output (the `.spx` format). Gap: `-timeout` is not enforced, since every read is local. Midget check of the output. |
| `networksetup` (`path:/usr/sbin/networksetup`) | `cmd/src/networksetup.rs`, `cmd/src/network.rs`, `cmd/src/bin/networksetup.rs` | `-listallnetworkservices`, `-listallhardwareports`, `-getairportpower`, `-setairportpower`, `-getairportnetwork`, `-setairportnetwork`, `-listpreferredwirelessnetworks`, `-removepreferredwirelessnetwork` (through the network service: status, enable, join, forget); `-getdnsservers`, `-getsearchdomains` (from the resolver file); `-getwebproxy`, `-getsecurewebproxy`, `-getsocksfirewallproxy`, `-getproxybypassdomains` (from the environment). Changes outside Wi-Fi are refused by name. | Gap: DNS and proxy settings cannot be set. The network service has no DNS or proxy surface, so `-setdnsservers` and the `-set...proxy` options are refused. Gap: the resolver file is system-wide, so every service reports the same DNS servers. Gap: no `-getinfo`, `-getcomputername` or service create, rename and order options. Midget check of Wi-Fi power and join against the live service. |
| `scutil` (`path:/usr/sbin/scutil`) | `cmd/src/scutil.rs`, `cmd/src/bin/scutil.rs` | `--get` and `--set` for `ComputerName`, `LocalHostName` and `HostName` (the local host name), with the `bifrost_core::host_name` rules; names saved to `host-names` in the shell's state directory (`BIFROST_SHELL_DIR`, default `/var/lib/springboard`). `--dns` prints the resolver file as a resolver dump. `--proxy` prints the environment's proxy settings as a dictionary. | Gap: the kernel host name is not changed, because that needs the privileged service (`privd`), which the command line cannot reach yet. Gap: `HostName` is the local host name; there is no separate domain name. Gap: `--dns` and `--proxy` only read. Gap: `-r`, `-w`, `--nc`, `--renew` and `--prefs` are refused. Midget check of the saved names. |

## Shared code

- `bifrost_core::sysinfo` now holds `size_label`, `clock_label` and `usb_speed`. The System Information app
  (`apps/sysinfo-desktop`) uses them from there, and its copies and their tests were removed (app 0.3.0). The
  `system_profiler` tool uses the same functions and the same `Report` readers.
- The models are in `bifrost-core`, not `bifrost-ui`, because the readers that fill them live there and `bifrost-ui`
  sits above core. Moving them to `ui` would invert the crate graph. The app and the command line already share them.
- `bifrost-proto` is a new dependency of `bifrost-cmd` (for the network service's frames). `proto` depends only on
  `ui` and `wire`, so the graph has no cycle.

## Midget checks still needed

- `sw_vers` and `system_profiler -detailLevel full` output on the live system.
- `networksetup -getairportpower Wi-Fi`, `-setairportpower Wi-Fi off` then `on`, and `-getairportnetwork Wi-Fi` against the
  running network service (`is.olibuijr.net`).
- `scutil --set ComputerName ...` then `scutil --get HostName`, after a shell restart.

## Adopt and delete

- The System Information app's copies of `size_label`, `clock_label` and `usb_speed` (and their four tests) were
  deleted from `apps/sysinfo-desktop/src/model.rs`. The app's imports now come from `bifrost_core::sysinfo`.
- No other caller existed. `core/src/host_name.rs` and `core/src/proxy.rs` had no shell or app caller before this
  slice; the command line is their first.
