# Wiring: network path service (`network`)

Matrix row: `docs/parity-matrix/data/network.json`, feature "Network path monitoring" (Network, NWPathMonitor, NWPath,
NWInterface). Model: `springboard/core/src/net_path.rs` (unchanged). This round adds the shell side and the SDK client.

## What is wired now

- **Shell service `network`** (`springboard/src/shell/services/network_path.rs`, registered in `services/mod.rs`).
  It reads the desktop network state the shell already gets from `netd` (`shell::desktop_network::Snapshot`) and
  turns it into a `bifrost_core::net_path::Path`. Only connected interfaces count as links. An address family is up
  when a connected interface has one. Interface kinds come from the netd types: `ethernet` is wired, `wifi` is Wi-Fi.
- **Methods:**
  - `path` returns `{"status": "satisfied" | "unsatisfied" | "requiresConnection", "interfaces": [{"name", "kind": "wired" | "wifi" | "cellular" | "loopback" | "other"}], "expensive", "ipv4", "ipv6"}`.
- **Event:** `pathUpdate`, same body as `path`. Sent to every running app on the shell tick that sees a real change
  (`PathMonitor::update` filters repeats). The first tick after start counts as a change from offline.
- **SDK client** (`springboard/sdk/src/network_path.rs`, `bifrost_app::network_path`):
  - `request_path(ctx) -> u32` sends `network.path`.
  - `decode_reply(Result<Json, String>) -> Result<Path, String>`.
  - `path_event(service, event, body) -> Option<Result<Path, String>>` for `App::service_event`.
  - Re-exports `Path`, `Status`, `Interface`, `IfaceKind`, `PathMonitor`.
- The JSON codec (`path_to_json`, `path_from_json`) lives in the shell service and is shared by the SDK, so the two
  sides cannot drift. The wire protocol (`springboard/proto`) is unchanged; the service channel carries the calls.

## How a user reaches it

There is no screen or menu item. A user reaches it through an app that calls `network_path::request_path` and
handles `pathUpdate`. No shipped app does this yet.

## How to verify on the test PC (Midget)

1. Build and publish as usual (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`).
2. Run a test app (or any app with the hooks above) that calls `request_path` at start and logs the decoded path.
   Expect `status=satisfied`, one `wired` interface, `ipv4=true`, `expensive=false`.
3. Unplug the Ethernet cable (Midget uses wired VLAN 20). Expect a `pathUpdate` with `status=unsatisfied` and an
   empty interface list. Plug it back in and expect `satisfied` again. Repeated identical states send nothing.
4. On Wi-Fi only, the path shows one `wifi` interface and `expensive=false`.

Host tests (not run in this round, per the no-build rule for this worktree): `cargo test -p springboard
network_path` covers the shell mapping and codec, and `cargo test -p bifrost-app network_path` covers the client.

## REA evidence

- `ev_e342d5724becff7d20b87de7d463be4d35a34a9c1e3fb3a1d89d196d0ee68cd0`: NWPathMonitor page (rendered). Monitor
  init forms, `start(queue:)`, `currentPath`, `pathUpdateHandler`, `cancel()`.
- `ev_85e659002f637462b75830f0f221cdcbf5af190163c87ea27bfcc6206be0ffe3`: NWPath page (rendered). `status`,
  `usesInterfaceType`, `availableInterfaces`, `supportsIPv4`, `supportsIPv6`, `supportsDNS`, `isConstrained`,
  `isExpensive`, `isUltraConstrained`, `unsatisfiedReason`.
- The `docc.py` JSON endpoint returned 404 for both paths, so the rendered pages are the evidence used.

## Remaining gaps

- **Constrained (Low Data Mode):** the model has no `constrained` flag and netd has no source for it. Not reported.
- **Interface type filter:** `usesInterfaceType` is not in the SDK. Apps read `interfaces` and filter themselves.
- **Unsatisfied reason, link quality, gateways, DNS support:** not in the model. Not reported.
- **Reachability of a named host:** not implemented. The path says whether the machine has a route, not whether a host
  answers. Needs a probe API, which is a separate change.
- **No user-facing surface:** no Control Center or Settings row reads the path yet.
- **Nothing calls the service in a shipped app.** Verification needs a test app.
- **Cellular:** netd has no cellular path (see the matrix row for Core Telephony), so `cellular` never appears on PC.
- **Live state is pull plus shell tick.** The path updates when netd's snapshot changes, at the shell tick rate. There is
  no separate push from netd.
