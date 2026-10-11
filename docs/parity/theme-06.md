# Theme 06: packet tunnel provider, app proxy provider and custom VPN

Wave 2 closes part of the VPN and proxy gap. The model is in two pure Rust modules in `springboard/core/src/`
(std only, no unsafe, no network, no interface). It does not build a system VPN, a network extension host or a
PAC script engine. Nothing in the shell or the settings pages calls it yet, so every row below is `partial`.

## What was built

- `springboard/core/src/tunnel.rs`
  - `TunnelSession`: life cycle `Disconnected -> Connecting -> Connected`, `Connected <-> Reasserting`, and
    `Disconnecting -> Disconnected`. Illegal calls return `IllegalTransition` and leave the state unchanged.
    It records the stop reason, the last error, and the number of connections (renegotiation is not counted).
  - `TunnelError`: configuration, unreachable server, rejected credentials, timeout, cancelled, internal.
  - `VpnProfile` and `VpnProtocol` (IKEv2, IPsec with IKEv1, provider-defined tunnel): validation of the name,
    the server (no scheme or path) and the port. The profile holds no secrets.
  - `Ipv4Route` (CIDR parse, masking, containment) and `RoutePolicy`: include and exclude rules over a default
    action; the longest matching prefix wins, and the later rule wins on a tie. Full-tunnel and split-tunnel
    layouts both work.
  - `parse_ipv4` and `classify_packet`: reads an IPv4 header from bytes, checks the version, lengths and the
    ones' complement header checksum, and returns the route decision for the destination.
- `springboard/core/src/proxy.rs`
  - `ProxyEndpoint` (validated host and port), `ProxySettings` (HTTP, HTTPS and SOCKS endpoints, bypass
    patterns, simple host names, auto-configuration URL stored but not evaluated).
  - `ProxySettings::bypasses` (exact host, domain with subdomains, `*.domain` wildcard) and
    `ProxySettings::choose(scheme, host)`, which returns the ordered proxies to try, or `[Direct]`.
- `springboard/core/src/lib.rs`: one `pub mod` line each for `tunnel` and `proxy`.
- Tests: 9 in `tunnel.rs` and 6 in `proxy.rs`. They were not run (no builds in this round).

## Apple APIs mirrored

Official pages:
- `NEPacketTunnelProvider` (DocC: `https://developer.apple.com/documentation/networkextension/nepackettunnelprovider`).
  Life-cycle methods `startTunnel`, `stopTunnel`, `cancelTunnelWithError`; `packetFlow`; `virtualInterface`;
  `createTCPConnectionThroughTunnel`, `createUDPSessionThroughTunnel` (deprecated).
- `NEAppProxyProvider` (DocC: `https://developer.apple.com/documentation/networkextension/neappproxyprovider`).
  `startProxy`, `stopProxy`, `cancelProxyWithError`, `handleNewFlow`, `handleNewUDPFlow`.
  Only the life-cycle shape is mirrored in `TunnelSession`; flow handling is not.
- `CFNetworkCopyProxiesForURL` and the global proxy configuration article: the DocC path
  `cfnetwork/global-proxy-configuration` returned HTTP 404, so the article's content was not read. The
  `choose` shape follows the matrix row's description only.
- `SCDynamicStoreCopyProxies`: the matrix row cites it; no DocC page was fetched in this round.
- Routing article ("Routing your VPN network traffic"): cited in the matrix; the include and exclude model is
  a simplified version of the routes that `setTunnelNetworkSettings` takes.

## REA evidence

- `ev_1751e315f71f3fcca0c95df81118c97cf7b38a8e9ffadc6b731a0978893fc2d7`: rendered page for
  `https://developer.apple.com/documentation/networkextension/nepackettunnelprovider` (title, overview,
  life-cycle methods, packet flow property, the subclass rules).
- `ev_9d1a28b576df09f4bc09965b4549009ae8a7724778f182f3443c8b49d3216a45`: rendered page for
  `https://developer.apple.com/documentation/cfnetwork/global-proxy-configuration`. The page is the site's
  "page you're looking for can't be found" page, so it is not evidence of the article's content. It is recorded
  here as-is.

## Matrix rows closed

| framework slug | feature (as in data/<slug>.json) | new status | BifrOSt path |
|---|---|---|---|
| network-extension | Packet tunnel provider and app proxy provider (custom VPN) | partial | springboard/core/src/tunnel.rs (`TunnelSession`, `parse_ipv4`, `classify_packet`) |
| network-extension | Routing VPN traffic (include and exclude) | partial | springboard/core/src/tunnel.rs (`RoutePolicy`, `Ipv4Route`) |
| network-extension | VPN configuration (personal VPN, IKEv2/IPsec) | partial | springboard/core/src/tunnel.rs (`VpnProfile`, `VpnProtocol`) |
| network-extension | Error types (NEVPNError, NETunnelProviderError, NERelayClientErrorDomain) | partial | springboard/core/src/tunnel.rs (`TunnelError`); no relay error domain |
| system-configuration | Network connections (start, stop, status) for VPN and PPP | partial | springboard/core/src/tunnel.rs (`TunnelSession`); no PPP, no system configuration store |
| system-configuration | Proxy settings | partial | springboard/core/src/proxy.rs (`ProxySettings`) |
| cfnetwork | Global proxy configuration | partial | springboard/core/src/proxy.rs (`ProxySettings::choose`); no script evaluation |

## Gaps that remain

- No system extension host, no `NEAppExtensionConfiguration`, and no provider process. The tunnel model is
  not connected to an interface, a socket or the packet path.
- No IKEv2 or IPsec implementation. `VpnProtocol` only names the protocols.
- No settings page reads or writes `VpnProfile`, `RoutePolicy` or `ProxySettings`. The VPN page in
  `springboard/src/shell/pages.rs` is still a static placeholder, and the Network page has no proxy row.
- Proxy auto-configuration scripts are not evaluated.
- No DNS proxy, no relay configuration (HTTP/2 and HTTP/3), no flow API for app proxies.
- The `choose` order and the bypass matching are our own rules, checked only against our tests. They are not
  verified against the global proxy configuration article, which was not available.
- Nothing has been compiled or run in this round. Run `cargo test -p <core crate>` once builds are allowed.
