# Wave 5: Network (connections, listeners, transport options, service discovery)

Branch `parity/w5-network`, on top of `parity/integration` (wave 4, 8200cb0f). Pure std, no new crates, no
unsafe, no socket is opened by any new code. Builds and tests were not run in this wave (no-build rule); the
unit tests are written and must be run in the integration build.

## What was built

- `springboard/core/src/net_connection.rs` (new): `Endpoint` (host and port, DNS-SD service, UNIX path) with a
  `host_port` parser (IPv4 and names, bracketed IPv6, port 1 to 65535); `Parameters` (`tcp()`, `tls()`, `udp()`)
  with `TcpOptions` (no delay, keepalive idle, connection timeout) and validation (TCP options refused on UDP,
  zero timeouts refused); `Connection` with the state machine setup, preparing, ready, waiting, failed, cancelled
  (`handle` returns false for events that do not apply, failed and cancelled are terminal); `Listener` with the
  states setup, ready, waiting, failed, cancelled, a connection limit (`set_connection_limit`, `accept`).
- `springboard/core/src/dns_browse.rs` (new): `ServiceType::parse` (`_name._tcp` or `_udp`, RFC 6335 name rules,
  lower-cased) and `dns_name(domain)`; `Browser` for one service type and domain, with `browse_reply` (add and
  remove, case-insensitive, instance names up to 63 bytes), `resolve_reply` (host, port, TXT record from the existing
  `txt_record::TxtRecord`), `instance`, `instances` (sorted without regard to case) and `full_name`. It reuses the
  DNS-SD wire model in `springboard/core/src/dnssd.rs` only through its shared types; it sends no query.
- `springboard/sdk/src/network_connection.rs` and `springboard/sdk/src/network_browser.rs` (new): app-facing
  re-exports of the two core modules. Both registered in `springboard/sdk/src/lib.rs`; the two core modules are
  registered in `springboard/core/src/lib.rs`.

Tests: `#[cfg(test)] mod tests` in each core module (6 in `net_connection.rs`, 6 in `dns_browse.rs`), with expected
values worked out by hand from the rules above.

## Apple API names mirrored

- `NWConnection` (state, `start`, `restart`, `cancel`, `endpoint`, `parameters`), `NWConnection.State` cases
  `setup`, `preparing`, `ready`, `waiting(_:)`, `failed(_:)`, `cancelled`.
- `NWEndpoint` (`hostPort`, `service`, `url`, `unix`), `NWEndpoint.Host`, `NWEndpoint.Port`.
- `NWParameters` (`tls`, `tcp`, `udp`, `init(tls:tcp:)`), `NWProtocolTCP.Options` (`noDelay`,
  `enableKeepalive`, `keepaliveIdle`, `connectionTimeout`).
- `NWListener` (`init(using:on:)`, `start`, `cancel`, `newConnectionLimit`, `InfiniteConnectionLimit`), `NWListener.State`.
- `NWBrowser` (`init(for:using:)`, `start`, `cancel`, `browseResults`, `NWBrowser.State`, `NWBrowser.Result`
  change set and metadata).

Official pages read with `python3 -I tools/parity/docc.py`: `network/nwconnection` (and `nwconnection/state-swift.enum`),
`network/nwlistener` (and `nwlistener/state-swift.enum`), `network/nwbrowser` (and `nwbrowser/state-swift.enum`,
`nwbrowser/result`), `network/nwparameters`, `network/nwendpoint` (and `nwendpoint/host`, `nwendpoint/port`),
`network/nwprotocoltcp/options`.

## REA evidence

- Network framework landing page (`https://developer.apple.com/documentation/network`):
  `ev_e87e20db545767ee8ef5b283ccc631299c53f31891374dd7b15b082215f9de13`.

## Matrix rows closed

These four rows change status in `docs/parity-matrix/data/network.json`. The lead updates the matrix centrally;
the new status proposed here is `partial`, because the models exist but no app can open a socket through them yet.

| Framework (slug) | Feature (exact, from data/network.json) | New status | BifrOSt path |
|---|---|---|---|
| network | Connections (NWConnection) to a remote endpoint | partial | springboard/core/src/net_connection.rs; springboard/sdk/src/network_connection.rs |
| network | Listeners for incoming connections | partial | springboard/core/src/net_connection.rs (Listener); springboard/sdk/src/network_connection.rs |
| network | TCP, UDP and IP protocol options | partial | springboard/core/src/net_connection.rs (Parameters, TcpOptions); springboard/sdk/src/network_connection.rs |
| network | Service discovery on the local network | partial | springboard/core/src/dns_browse.rs; springboard/sdk/src/network_browser.rs |

## Not done in this wave

- No transport: no app-facing call opens a TCP, UDP or TLS connection, binds a listener, or sends a DNS-SD query.
  The shell's own TCP and TLS (`springboard/src/deildu/http.rs`) and its UDP beacon (`springboard/src/net/share.rs`)
  are unchanged.
- No TLS handshake options, client identities, QUIC, or framers.
- Nothing is wired into apps or the shell, and nothing has been tested on the VM or Midget.
