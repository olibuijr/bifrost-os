# Network Extension parity

Status of the Network Extension rows in `docs/parity-matrix/data/network-extension.json`. Rows stay `partial` until a
test runs on Midget; nothing here is `parity` yet.

## Personal VPN configuration (NEVPNManager shape)

- Model: `springboard/core/src/vpn_manager.rs` (`VpnManager`, `VpnStatus`), built on the tunnel models in
  `springboard/core/src/tunnel.rs` (`VpnProfile`, `VpnProtocol`, `TunnelState`, `TunnelError`).
- App API: `springboard/sdk/src/vpn_configuration.rs`, re-exported through `bifrost_app`. The shell uses `bifrost_core`
  directly.
- Flow matches the reference: set the protocol configuration, enable, save. Enabling needs a saved configuration
  the system can run. Remove clears the configuration and disables the VPN.
- Mapping to NetworkManager: `networkmanager_keyfile()` renders a keyfile with `[connection]` (`id`, `type=vpn`,
  `autoconnect=false`) and `[vpn]` (`service-type`, `remote`, `port`). IKEv2 uses the strongSwan plugin service and
  IPsec/IKEv1 uses the libreswan plugin service. The plugin service names come from upstream NetworkManager.
- Decisions:
  - `Tunnel` profiles belong to an app provider, so they have no system keyfile and cannot be enabled through this
    manager.
  - `on_demand` is applied by the system layer when a network appears. The keyfile always sets `autoconnect=false`.
  - The port is written under `[vpn]` for the system layer to read. The plugin keys do not carry one, so the port
    key is a BifrOSt convention.
  - The profile holds no secrets. Credentials stay with the caller (keychain).
- Not built yet: saving and loading the configuration to disk, the system layer that writes the keyfile and brings the
  tunnel up, and the VPN settings page. `PageId::Vpn` (`desktop_vpn_page` in `src/shell/pages.rs`) now lists profiles from
  `desktop_network::vpn_profiles()` with `vpn_summary`, but that function returns an empty list because nothing stores
  profiles yet, and the page says the tunnel client is not built. Nothing in the shell uses `VpnManager` (2026-10-13).
- Tests: unit tests in `vpn_manager.rs`. They run on the host and change no network state.
