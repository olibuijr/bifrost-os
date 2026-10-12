# Wire 2: Bluetooth service browser and DNS-SD browse references

Branch `wire2/bluetooth-net` on top of `main` (release 0.5.37). This round wires two wave-6 models into the shell's
service channel and the Settings Bluetooth page: the Bluetooth service browser (`bifrost_core::bt_service_browser`)
and the DNS-SD operation reference lifecycle (`bifrost_core::dnssd_ref`, with `bifrost_core::dns_browse` for the
browse state). No wire protocol change (`springboard/proto` is untouched); no new crate; std only; no unsafe.
Nothing here has been built or run on a host or on Midget yet (the lead builds once).

## What is now wired

### 1. Bluetooth service browser (`springboard/src/shell/services/bluetooth_service.rs`)

- New service method `services(addr)` on the `bluetooth` service. It checks the address against the devices the shell
  knows (an unknown address is an error), runs a `ServiceBrowser` over the records it has, and replies
  `{device, source, note, services: [{uuid, name}]}`.
- `source` is `"none"` and `services` is empty, because the shell does not run an SDP query on the radio yet. The
  `note` says so. This is the honest state: no hardware service data exists, and none is invented.
- `services_reply` takes the records as an argument, so the allow-list filtering is tested now and will apply as soon
  as a query feeds it. The UUID is written as hex (4, 8 or 32 digits for the 16-, 32- and 128-bit forms).
- Settings › Bluetooth › (a device) now has a `SERVICES` group with the row `Services: None listed` and the note as
  its footer (`springboard/src/shell/pages.rs`, `BluetoothDetail`).
- SDK client: `springboard/sdk/src/bluetooth_service.rs`: `services(ctx, addr)`, `services_from_reply`, and the
  `ServiceList` type.

### 2. DNS-SD browse references (`springboard/src/shell/services/network_path.rs`)

The `network` service keeps a `dnssd_ref::Registry` and up to 16 open browses. Each browse owns a `Ref<Change>` and a
`Browser` for one service type in the `local.` domain.

| Method | Params | Reply |
|---|---|---|
| `dnssdBrowse` | `"_http._tcp"` | `{ref, name: "_http._tcp.local.", domain, source: "none"}` |
| `dnssdInstances` | `ref` (number) | `{ref, source, instances: [name...]}` |
| `dnssdNext` | `ref` | `{added}`, `{removed}` or `null` when nothing is queued |
| `dnssdClose` | `ref` | `{dropped}`; the number is retired and never reused |

- Reference numbers start at 1 and are never reused. A closed or unknown number is an error. A malformed service type
  (no leading `_`, bad transport, and so on) is refused with the parser's message.
- The shell has no DNS-SD daemon, so no reply is ever pushed into a browse. Every browse returns zero instances, with
  `source: "none"`. The daemon layer that will call `Browser::browse_reply` and `Ref::push` is not written.
- SDK client: `springboard/sdk/src/network_path.rs`: `dnssd_browse`, `dnssd_instances`, `dnssd_next`, `dnssd_close`,
  `reference_from_reply` and `instances_from_reply`.
- No Settings row was added for DNS-SD. Settings › Network has no source of names yet, so a row would only say
  "none".

## How a user reaches it

- Bluetooth: Settings › Bluetooth › pick a device under My Devices › the `SERVICES` group shows `None listed` and the
  note. Apps reach the list with `bifrost_app::bluetooth_service::services`.
- Network: no Settings surface. Apps call `bifrost_app::network_path::dnssd_browse` and friends.

## How to verify on Midget (test PC)

1. Build the fast profile and push it (`tools/midget-push-shell.sh`), then `bifrost pc midget state` to check the shell
   is up.
2. Pair a device, open Settings › Bluetooth, tap the device. The `SERVICES` group must read
   `Services` / `None listed` with the footer above. The device page is only reached from My Devices, so an unpaired nearby device has no such page.
3. From an app, call `services` on the `bluetooth` service for a listed address. Expect `source: "none"` and an empty
   `services` array. For an address not on the list, expect an error.
4. From an app, call `dnssdBrowse` with `"_http._tcp"`, then `dnssdInstances` (expect `[]`), `dnssdNext` (expect
   `null`) and `dnssdClose` (expect `dropped: 0`). A second `dnssdInstances` on the closed ref must fail.
5. Use `bifrost pc midget tap-window` for every tap in the Settings window (window-local pixels, not `tap`).

## REA evidence

Official pages were read with `tools/parity/rea-page.sh` (captures under the REA evidence store). The DocC JSON
(`tools/parity/docc.py`) returned HTTP 404 for `documentation/dnssd/dnsservicebrowse`,
`documentation/iobluetoothui/iobluetoothserviceBrowsercontroller`, `documentation/dnssd` and
`documentation/iobluetoothui` in this session, so the rendered captures are the only evidence recorded here:

- https://developer.apple.com/documentation/iobluetoothui: `ev_debf04a25be6b4d77c60461b9600cde1f7c658dde938cd1738e5903fd3178d46`
- https://developer.apple.com/documentation/dnssd: `ev_a6941845491720961cd9089b04092027f7ed7daf06ad96eb6f0e4cb6026ec124`
- https://developer.apple.com/documentation/dnssd/dnsservicebrowse: `ev_4a7ca99a4e70f60b89cc76fb58980b7ab181d1b7ee90f0004865a773704c7ec4`
  (the rendered page was "page can't be found", so it is not a usable reference; the DocC entry is still needed)

Gap in the evidence: the reference pages for the service browser controller and the DNS-SD reply callbacks must be
re-read when the DocC mirror answers again before the rows move past `partial`.

## Remaining gaps

- No SDP query on the radio: the service list is always empty. Until a query exists, the Bluetooth row stays `partial`.
- No DNS-SD daemon: browses never receive instances. The shared-connection form (`DNSServiceCreateConnection`),
  dispatch-queue delivery, and `RefSockFD` as a real descriptor are not modelled (as in `dnssd_ref`).
- The service browser's panel (`runModal`, sheet, `getRef`) is not drawn; only the state and the allow list are wired.
- Settings › Network has no DNS-SD row.
- Apple's `IOBluetoothServiceBrowserController` behaviour (choosing a service, the sheet) is not matched, and the
  matrix row says so.

## Matrix rows this round touches (not edited here)

Feature strings are as written in `docs/parity-matrix/data/<slug>.json`. All stay `partial` until the daemon and the
radio query exist, and the Midget check above passes.

| Slug | Feature | BifrOSt path |
|---|---|---|
| iobluetooth-ui | Service browser (IOBluetoothServiceBrowserController) | springboard/core/src/bt_service_browser.rs, springboard/src/shell/services/bluetooth_service.rs (services), springboard/sdk/src/bluetooth_service.rs |
| dnssd | Browse for services by type (DNSServiceBrowse) | springboard/core/src/dns_browse.rs, springboard/src/shell/services/network_path.rs (dnssdBrowse, dnssdInstances), springboard/sdk/src/network_path.rs |
| dnssd | Connection and reference lifecycle (DNSServiceRef, ProcessResult, RefSockFD, SetDispatchQueue, RefDeallocate) | springboard/core/src/dnssd_ref.rs, springboard/src/shell/services/network_path.rs (dnssdNext, dnssdClose) |
| network | Service discovery on the local network | springboard/core/src/dns_browse.rs, springboard/src/shell/services/network_path.rs, springboard/sdk/src/network_path.rs |
| foundation-networking-lowlevel | Bonjour: advertise and browse services | springboard/core/src/dns_browse.rs, springboard/src/shell/services/network_path.rs (browse only; no advertise) |
