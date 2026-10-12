# Wave 4: Network parity (path monitoring, WebSocket framing)

Two gap rows of the Network framework are closed in pure Rust (std only, no new crates, no unsafe, no sockets).
Both are pure models with unit tests. Neither is wired into the shell or into a running app yet.
No build or test run was made for this wave: the code was written and checked by reading only.

## What was built

### Network path monitoring (row was `os`, now `partial`)

- `springboard/core/src/net_path.rs`: the path model. `Path::new(interfaces, ipv4, ipv6)` derives the status:
  `Unsatisfied` with no non-loopback link, `RequiresConnection` with a link but no address (DHCP still
  running), `Satisfied` with a link and an address. Interfaces are sorted by preference (wired, Wi-Fi,
  cellular, other, loopback). `expensive` is true when the primary link is cellular.
  `PathMonitor::update` returns the new path only when it differs from the last one.
- `springboard/sdk/src/network_path.rs`: app-facing re-export as `bifrost_app::network_path`.
- The system layer (`springboard/netd`, `springboard/src/net/wifi.rs`) still does not push link state into the
  model. That is the missing step to make the row `parity`.

Official API names mirrored: `NWPathMonitor` (`init`, `start(queue:)`, `currentPath`, `pathUpdateHandler`,
`cancel`), `NWPath`, `NWInterface` (`Interface.Type`). Doc: https://developer.apple.com/documentation/network/nwpathmonitor

### QUIC, WebSocket and custom framers (row was `gap`, now `partial`)

- `springboard/core/src/websocket.rs`: RFC 6455 frame codec. `encode(frame, mask)` writes the 2, 4 or 10 byte
  header and masks client frames. `decode(buf)` returns `Ok(None)` when the frame is incomplete, else the
  frame and the bytes used. It rejects reserved bits, reserved opcodes, fragmented control frames and control
  payloads over 125 bytes. `accept_key(key)` computes `Sec-WebSocket-Accept` (SHA-1 and base64 are private to
  the module).
- `springboard/sdk/src/websocket.rs`: app-facing re-export as `bifrost_app::websocket`.
- Not built: QUIC, the message framer protocol (`NWProtocolFramer`), the opening handshake I/O, the connection
  itself. Those need sockets, so they are outside this wave.

Official API names mirrored: `NWProtocolWebSocket` (`Opcode`, `Metadata`, `Response`, `CloseCode`). Doc:
https://developer.apple.com/documentation/network/nwprotocolwebsocket

## Tests

- `net_path.rs`: 7 tests (offline, wired sorted first, link without address, loopback only, IPv6 only,
  cellular expense, monitor reports changes only).
- `websocket.rs`: 7 tests, including the RFC 6455 accept key example (`dGhlIHNhbXBsZSBub25jZQ==` gives
  `s3pPLMBiTxaQ9kYGzzhZRbK+xOo=`), the SHA-1 test vector for `abc`, base64 vectors, the RFC 6455 masked
  "Hello" frame (encode gives back the same bytes), a 126-byte extended length header, incomplete input, and
  three error cases.

## REA evidence (official pages, rendered)

- `ev_69f2e0c94d198a48a302f45e6f8fb6d0209770d4f30fd05315ba3730b1528c74`: Network framework landing page.
- `ev_738fac84c01a03ac151bebdd2eabd002d9244c482895b3f2c0fddac8fcb66d6d`: NWPathMonitor.
- `ev_2f64d26aec2f87de982be4df3258078ee8b602195ac680de07b1f42d66b01282`: NWProtocolWebSocket.

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| network | Network path monitoring | partial | springboard/core/src/net_path.rs; springboard/sdk/src/network_path.rs |
| network | QUIC, WebSocket and custom framers | partial | springboard/core/src/websocket.rs; springboard/sdk/src/websocket.rs |

## Not done in this wave

- Service discovery (NWBrowser, Bonjour) is left alone. `springboard/core/src/dnssd.rs` and `dns_message.rs`
  already cover part of it, and the mDNS transport is missing.
- The path model needs a live feed from netd before the row can go to `parity`.
- No build, no test run, no host preview. Check with `cargo test -p bifrost-core net_path websocket` before the
  next release round.
