# Wiring: Remote app (service discovery and network path)

Branch `wire/app-remote`. Remote is now `0.2.0` (`springboard/apps/remote/`). It wires two parity models into the
app, which previously used none of them: service discovery (DNS-SD browse) and network path monitoring. Remote's
sharing service itself is unchanged.

## What is now wired

| Matrix slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt model | Where it runs in Remote |
|---|---|---|---|
| `network` | Service discovery on the local network | `bifrost_app::service_discovery::ServiceBrowser` (SDK, over `bifrost_core::dnssd` messages) | `src/nearby.rs`, `Nearby::poll`: a non-blocking UDP socket on an ephemeral port sends a PTR query for `_bifrost-remote._tcp.local` to 224.0.0.251:5353 every 5 s and folds the replies in |
| `network` | Network path monitoring | `bifrost_app::network_path::{Path, Interface, IfaceKind, Status}` | `src/network.rs`: reads `ip -j link`, `ip -j -4/-6 address` and builds a `Path`; the window shows "Connected by Ethernet / Wi-Fi / mobile data (metered)", "Getting an address", or "No network connection" |
| `system-configuration` | Reachability of hosts and addresses | partial: the path model only (no per-host probe) | the path status line; a host probe is not built |

The app's own list model (`nearby::rows`) is pure: it maps each browsed instance to a title and a URL
(`http://ip:port`, IPv6 bracketed, a host name when no address came back, `Resolving...` until a reply carries one).

## How a user reaches it

Open Remote (App Store, `is.olibuijr.remote.desktop`). The window has two new sections below the sharing controls:

- **Network**: one line for the connection state.
- **Nearby computers**: up to four rows of other computers that announce `_bifrost-remote._tcp`. Tap a row to
  select it; the line under the list shows `Address: http://...`. Tap it again to clear.

## How to verify on the test PC

Test on Midget (the test PC): `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`, then
`bifrost pc midget open-app is.olibuijr.remote.desktop`.

1. The Network line reads "Connected by Ethernet" (or Wi-Fi) on the lab network. Unplug the cable or disable Wi-Fi
   and it reads "No network connection" within 5 s.
2. Nearby computers reads "No remote computers found on this network." when no other computer advertises the
   service. A DNS-SD advertiser on the same network (for example `avahi-publish -s "Test" _bifrost-remote._tcp 12000`
   on Titan) makes a row appear within about 5 s; tapping it shows `Address: http://<ip>:12000`.
3. `cargo test -p bifrost-remote` (run by the lead, not by this round) covers `nearby::tests` and `network::tests`.

## REA evidence

Official reference for the browse and path models:

- `ev_c722e25ff58407ae99f1d3b45cccc5ae727cb29f8623fd202ddaffc74326ee2f`: NWBrowser (browse results, state, descriptor), rendered at developer.apple.com/documentation/network/nwbrowser
- `ev_cf9cd1f77f078073dd51c687ed786bbfe73626651a4fb6412a27a0a3edc9d04b`: NWPathMonitor (current path, path update handler, ethernetChannel), rendered at developer.apple.com/documentation/network/nwpathmonitor

`docc.py` output for the same pages: `network/nwbrowser`, `network/nwpathmonitor`.

## Remaining gaps

- **No advertising.** Remote does not announce itself, so no other computer lists it. Advertising needs UDP port 5353
  bound next to the system mDNS responder (which needs SO_REUSEADDR, which std cannot set; no new crates or unsafe).
  Needs a netd or SDK service that owns the advertisement. This is the main gap: the browse works, but today a
  Remote only finds peers that another responder advertises.
- **No live link events.** The path is re-read every 5 s with `ip`. The NWPathMonitor-style callback needs netd to push
  link state to apps; netd only publishes Wi-Fi status to the shell today.
- **Reachability is not probed.** The window reports the path, not whether a given computer answers. A per-host
  probe (`SCNetworkReachability` in Apple's model) would need a non-blocking connect with a timeout and a result
  row; not built.
- **Selection is by index.** A row selected before the list changes keeps its index and may point at another computer.
- **Snapshot golden.** `tests/snapshots/remote-{light,dark}.png` no longer match the new layout. The lead re-blesses
  them with `BIFROST_BLESS=1` after building.
- **Dependency.** Remote now depends on the in-repo `bifrost-core` path crate, to name the mDNS `Message` type. No
  third-party crate was added.
