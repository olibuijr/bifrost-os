# Wiring wave 5: Wi-Fi join, known networks and the VPN list (`wire5-wifi-join`)

Owner area: Settings > Network (Wi-Fi join, known networks, the VPN list) and the network path service. Matrix rows:
`network-extension` (Wi-Fi configuration, VPN configuration) and `core-wlan` (Join, select and forget; saved profiles).
No row moves to `parity`: each needs a test on Midget first.

## What is wired now

- **VPN configurations list (new).** Settings > Network now has a VPN row in the CONNECTIONS group. Its value is
  "None" until a profile exists, then "Configured". Tapping it opens the VPN page (`PageId::Vpn`,
  `springboard/src/shell/pages.rs`, `desktop_vpn_page`). The page lists each `bifrost_core::tunnel::VpnProfile` by name
  with a summary line from `desktop_network::vpn_summary` ("IKEv2, server:500", "IPsec, ...", "Tunnel, ...:443"). With no
  profile it shows a dimmed "No VPN Configurations" row and a footer that says profiles are not stored yet.
- **Profile source (stub).** `desktop_network::vpn_profiles()` returns an empty list. No code stores or loads a VPN
  profile yet, so the list is always empty on a PC. The shape is ready for the tunnel client to fill.

## Already wired before this wave (checked, not changed)

- **Join Other Network sheet** (SSID, security, password, hidden and 802.1X): `wifi_page.rs` `Sheet::other`, reached from
  Settings > Wi-Fi > "Other…" and sent as `WifiAction::JoinOther`.
- **Forget This Network** and **Auto-Join** per known network: `PageId::WifiDetail` (`settings.rs` tests
  `forget_network_from_the_detail_page`).
- **Known Networks** rows with the security type: `wire4-wifi-profiles.md`.
- **Network path monitoring** service: `wire-svc-network.md` (`services/network_path.rs`, unchanged this wave).

## How a user reaches it

Settings > Network > VPN. Join Other Network: Settings > Wi-Fi > Other…. Forget and Auto-Join: Settings > Wi-Fi > a known
network.

## How to verify on the test PC (Midget)

1. Publish and update Midget (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`).
2. Open Settings > Network. Expect a VPN row with the value "None" under CONNECTIONS.
3. Tap VPN. Expect the title "VPN", a CONFIGURATIONS header, the dimmed "No VPN Configurations" row, and the footer.
4. Settings > Wi-Fi > Other…: type a hidden SSID and a password, then Join. Expect the join to start.
5. Settings > Wi-Fi > a known network: Auto-Join off, then Forget This Network. Expect it to leave the list.

Host tests (not run in this round, per the no-build rule): `cargo test -p springboard vpn_summary_names_protocol`
and `cargo test -p springboard no_vpn_profiles_are_stored_yet` (both in `shell::desktop_network::tests`).

## REA evidence

None recorded this round. The VPN row reads the existing model from `springboard/core/src/tunnel.rs`, and no new Apple
page was read.

## Remaining gaps

- **No VPN profile storage and no tunnel client.** The list is empty, and "Add VPN" is not built. The `tunnel.rs` state
  machine and `VpnProfile::validate` are not connected to any connection.
- **Hotspot helper** (`springboard/sdk/src/hotspot_configuration.rs`) is not used by the shell or an app. Hotspot is
  still not wired.
- **Persistent network configuration as an SDK call.** `hotspot_configuration.rs` has no app-facing join call. The
  shell's `Cmd::Join` path is the only one.
- **Network path monitoring** is already served (see wire-svc-network). No Settings row reads the path.

## Matrix rows touched (status stays as it is)

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| network-extension | VPN configuration (personal VPN, IKEv2/IPsec) | springboard/src/shell/pages.rs (PageId::Vpn, desktop_vpn_page); springboard/src/shell/desktop_network.rs (vpn_profiles, vpn_summary); springboard/core/src/tunnel.rs |
| network-extension | Wi-Fi configuration (persistent networks, join) | springboard/src/shell/wifi_page.rs (Join Other sheet); springboard/src/shell/pages.rs (PageId::Wifi, KnownNetworks) |
| core-wlan | Join, select and forget | springboard/src/shell/wifi_page.rs; springboard/src/shell/settings.rs (WifiAction::Forget, SetAutoJoin) |
| network | Network path monitoring | springboard/src/shell/services/network_path.rs (unchanged) |
