# Wave 5: Foundation (Networking, Low-Level Utilities)

Wave 5 closes three gap rows of the Foundation networking framework with pure std models. Nothing here opens a socket,
starts a daemon or calls the shell; the apps and daemons own the transports. Code was not built or run in this wave.

## What was built

1. **WebSocket handshake and message assembly** (extends `springboard/core/src/websocket.rs`, re-exported from
   `springboard/sdk/src/websocket.rs`).
   - `handshake_request(host, path, key, protocols)` writes the RFC 6455 section 4.1 client request.
   - `check_handshake_response(response, client_key)` accepts only a 101 answer whose `Sec-WebSocket-Accept` matches the key.
   - `Assembler` joins fragmented text and binary frames, returns ping, pong and close at once (even mid-message),
     checks UTF-8 on text, and enforces a size limit. `Message` is the whole-message value; `frame_for` encodes one.
   - Mirrors `URLSessionWebSocketTask` (send, receive, `maximumMessageSize`, `sendPing`, `cancel(with:reason:)`,
     `closeCode`, `closeReason`, `URLSessionWebSocketTask.Message`).
2. **Local service discovery** (new `springboard/sdk/src/service_discovery.rs`, registered in `springboard/sdk/src/lib.rs`).
   - `ServiceBrowser` builds the PTR query for a type (`_http._tcp`, domain `local` by default), folds replies into
     `ServiceInstance` values (name, full name, host, port, addresses with CNAME chains, TXT), and removes an instance on
     a zero-TTL goodbye. Queries are ignored.
   - `ServiceAdvertiser` builds the announcement reply (PTR in the answers; SRV, TXT and A/AAAA in the additional section,
     cache-flush bit set on the unique records) and tells whether a received query asks for it.
   - Built on the existing DNS-SD wire model `springboard/core/src/dnssd.rs` (`query`, `answers_for`, `srv_rdata`,
     `txt_rdata`, `resolve_addresses`). The model does not own a socket on UDP 5353.
   - Mirrors `NetServiceBrowser` (search for services of a type in a domain) and `NetService` (publish a named service
     with port and TXT record).
3. **Cookie hand-off for URL loading** (extends `springboard/sdk/src/url.rs`).
   - `attach_cookies(&mut URLRequest, &HTTPCookieStorage, now)` sets the `Cookie` header from the jar.
   - `store_cookies(&HTTPURLResponse, &mut HTTPCookieStorage, now)` stores the `Set-Cookie` headers of a reply.
   - Mirrors the `HTTPCookieStorage` `setCookie(_:)`, `cookies(for:)` and `cookie header` path that `URLSession` uses
     when its session has a cookie store. The jar itself is the existing `springboard/sdk/src/http_cookies.rs`.

## Official API names mirrored

| Official name | Doc |
|---|---|
| `URLSessionWebSocketTask`, `URLSessionWebSocketTask.Message`, `send(_:)`, `receive()`, `sendPing(pongReceiveHandler:)`, `cancel(with:reason:)`, `maximumMessageSize` | https://developer.apple.com/documentation/foundation/urlsessionwebsockettask |
| `NetServiceBrowser` (service search) | https://developer.apple.com/documentation/foundation/netservicebrowser |
| `NetService` (`init(domain:type:name:port:)`, `setTXTRecord(_:)`, `addresses`) | https://developer.apple.com/documentation/foundation/netservice |
| `HTTPCookieStorage` (`setCookie(_:)`, `deleteCookie(_:)`, `removeCookies(since:)`, `cookieAcceptPolicy`) | https://developer.apple.com/documentation/foundation/httpcookiestorage |
| Foundation landing page (Networking: URL Loading System, Bonjour) | https://developer.apple.com/documentation/foundation |

## REA evidence

- `ev_84e039656a349a021dbf3c053ca6e6316a340b36a60fe331e4a115f87aff7df3`: Foundation landing page (Networking: URL Loading System, Bonjour; Low-Level Utilities: XPC, Streams, Sockets, and Ports).
- `ev_37390fb533cc2efbe213a1cafa464a1568c26e33ac7659157335abacf4dc2d01`: URLSessionWebSocketTask.
- `ev_ae116e1ca3f8b36667f237ee9d565f1cef2b857a70936f0546ff030ce3940a35`: NetServiceBrowser.

DocC summaries (`python3 -I tools/parity/docc.py`) were read for `foundation/urlsessionwebsockettask`, `foundation/netservice`
and `foundation/httpcookiestorage`.

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| foundation-networking-lowlevel | URL Loading: WebSocket and streaming tasks | partial | springboard/sdk/src/websocket.rs; springboard/core/src/websocket.rs |
| foundation-networking-lowlevel | Bonjour: advertise and browse services | partial | springboard/sdk/src/service_discovery.rs; springboard/core/src/dnssd.rs |
| foundation-networking-lowlevel | URL Loading: cookies, credentials, authentication challenges | partial | springboard/sdk/src/url.rs (attach_cookies, store_cookies); springboard/sdk/src/http_cookies.rs |

## Still open

- WebSocket: no connection task. An app must own the TCP or TLS socket, and the SDK does not run the read loop.
- Bonjour: no UDP 5353 socket, no multicast join, no probing or conflict handling, no `NetService` lifecycle callbacks.
- Cookies: `URLSession` does not call `attach_cookies` or `store_cookies` itself, so the session still keeps no cookies.
  `Expires` dates are ignored (see `http_cookies.rs`). Credentials and authentication challenges are not covered by this wave.
- Not built: no `cargo` or test run was done for this wave, so the new unit tests are unverified until the next build.
- Unchanged rows: XPC, Process and Pipe, Streams and Sockets, OperationQueue and RunLoop, upload and background sessions.
