# Wiring wave 4: Wi-Fi saved profiles, security type and details (`wifi-profiles`)

Owner area: Settings > Wi-Fi (Known Networks, the per-network sheet) and the Wi-Fi diagnostics line. The matrix rows are
`core-wlan` (Saved network profiles, Security types) and `network` (Network debugging and diagnostics, partial). The
rows stay `partial`: this round wires the Settings surface, and no row moves to `parity` until it is tested on Midget.

## What is wired now

- **Security type per saved network.** `desktop_network::security_of` maps NetworkManager's
  `802-11-wireless-security.key-mgmt` onto the shell's existing `Security` model (`springboard/src/net/wifi_config.rs`):
  empty or `none` is Open, `sae` is WPA3 Personal, `wpa-psk` is WPA2 Personal, `wpa-psk` with `sae` is WPA2/WPA3
  Personal, `wpa-eap` is WPA2 Enterprise. An unknown key list gives no type. The value is read for each saved network in
  `desktop_network::read` and stored in the new `SavedNetwork.security` field (`springboard/src/shell/mod.rs`).
- **Known Networks list** (`PageId::KnownNetworks`): each row now reads "WPA3 Personal", or "None, Auto-Join Off", or
  "WPA2 Personal, Incorrect Password" when a problem note is set. Forget, Auto-Join and priority were already wired
  (`WifiDetail`) and are unchanged.
- **Network details in the per-network sheet** (`PageId::WifiDetail`, for the connected network): a DETAILS group with
  Security, Interface (the Wi-Fi adapter name), Configure IPv4 (Automatic (DHCP) or Manually) and DNS Servers, taken from
  the desktop network state already read by the shell.
- **Diagnostics line**: a DIAGNOSTICS group with "Problem" appears on the sheet of a saved network that carries a failure
  note (for example "Incorrect Password").

## How a user reaches it

Settings > Wi-Fi (on a PC with a Wi-Fi adapter, Settings > Network > Wi-Fi) > Known Networks. Tap a network to open its
sheet, which shows Details, Diagnostics (when there is a problem), Auto-Join, Move Up / Move Down and Forget This Network.

## How to verify on the test PC (Midget)

1. Publish and update Midget as usual (`bifrost pc publish`, `bifrost pc midget ota` until `phase=accepted`).
2. Join a WPA2 network and a WPA3 network that are both saved (Settings > Wi-Fi > Other… to save one).
3. Open Settings > Wi-Fi > Known Networks. Expect each row to show its type: "WPA2 Personal" or "WPA3 Personal".
   A network saved with Auto-Join off shows the type followed by ", Auto-Join Off".
4. Open the connected network's sheet. Expect a DETAILS group: Security, Interface `wlan0` (or the adapter name),
   Configure IPv4 "Automatic (DHCP)", and the DNS servers from the lease.
5. Open a saved network that the shell has marked with a failure note. Expect a DIAGNOSTICS group with the note.
   On Midget this is rare, because the desktop path does not yet write failure notes (see gaps).
6. Forget a network from its sheet and expect it to leave the list.

Host tests (not run in this round, per the no-build rule): `cargo test -p springboard key_management_maps` covers the
mapping, and `cargo test -p springboard known_networks_name_the_security` covers the Known Networks rows and the details
sheet.

## REA evidence

No new REA evidence was recorded in this round. The Apple reference for these rows is the one already in the matrix
(`springboard/docs/` and `docs/parity/w5-core-wlan.md`, evidence `ev_296a757e840c0605e8bb1183bc903a57c3a6b669721fc714c43dd23958731193`,
the Core WLAN landing page). The behaviour wired here (security type per profile, Auto-Join, priority) follows that
page's profile vocabulary, and no page on the security enum was re-read in this round.

## Remaining gaps

- **Failure notes on the desktop path.** `desktop_network::read` sets every saved note to empty, so DIAGNOSTICS never
  shows on the PC path yet. The netd path (`springboard/src/bin/shell.rs`) gets notes from `ws.notes`, but its
  `SavedNetwork.security` is `None`, because `Known` in `springboard/src/net/wifi.rs` carries only an enterprise flag.
- **Enterprise EAP method.** The key list does not name PEAP or TTLS, so an enterprise network shows "WPA2 Enterprise"
  with no method. `Eap::Peap` is a placeholder used only for the label.
- **Diagnostics beyond the note.** No signal, channel, band, BSSID or link-quality rows: `CWChannel`, `CWChannelBand`
  and the RSSI history have no data in the desktop state. The event model in `springboard/sdk/src/wlan_events.rs` is not
  wired into the shell.
- **Keychain.** Passwords still live in `springboard.conf`; `springboard/sdk/src/wlan_keychain.rs` is not used by the shell.
- **WEP and WPA-only personal.** Not supported by the shell (see `Security`), so they show as Open or no type.
- **App access.** No SDK call exposes saved profiles, the details sheet or the security type to apps.

## Matrix rows touched (status stays partial)

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| core-wlan | Saved network profiles (CWNetworkProfile, CWMutableNetworkProfile) | springboard/src/shell/pages.rs (PageId::KnownNetworks, PageId::WifiDetail); springboard/src/shell/desktop_network.rs (saved list with security) |
| core-wlan | Security types (CWSecurity) | springboard/src/shell/desktop_network.rs (security_of); springboard/src/net/wifi_config.rs (Security) |
| core-wlan | Configuration (CWConfiguration, CWMutableConfiguration) | springboard/src/shell/pages.rs (WifiDetail: Auto-Join, Move Up/Down; unchanged this round) |
| network | Network debugging and diagnostics | springboard/src/shell/pages.rs (WifiDetail DIAGNOSTICS group, failure note) |
