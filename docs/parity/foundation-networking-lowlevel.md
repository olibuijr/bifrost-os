# Foundation (Networking, Low-Level Utilities) parity

Framework doc: https://developer.apple.com/documentation/foundation (Networking and Low-Level Utilities sections).
Matrix row file: `docs/parity-matrix/data/foundation-networking-lowlevel.json`.

## What was built

Two std-only modules in the app SDK, both pure Rust with no new crates and no `unsafe`. Neither touches the network or
the system layer.

### `URLComponents` and `URLQueryItem`: `springboard/sdk/src/url_components.rs`

Mirrors `URLComponents` (init(string:), queryItems, percentEncoded* properties, string) and `URLQueryItem(name:value:)`.

- `URLComponents::from_string` parses scheme, user, password, host (IPv6 literals in brackets), port, path, query items
  and fragment. It returns `None` for a port that is not a 16-bit number or an invalid authority.
- Parts are stored decoded. `percent_encoded_path`, `percent_encoded_query`, `percent_encoded_user`,
  `percent_encoded_password`, `percent_encoded_host` and `percent_encoded_fragment` give the RFC 3986 encoded form with
  per-part character sets. `&`, `=` and `+` are encoded inside query names and values.
- `query_items` is `None` when the URL has no `?` and an empty list for a bare `?`, as in Apple's API.
- `string()` builds the URL from the parts. Round-trips canonical URLs, including `file:///path` (an empty host is kept).

Behaviour that differs from Apple: decoding is lossy for non-UTF-8 bytes; `+` is never read as a space; empty query
segments (`a=1&&b`) are skipped; `string()` returns `String`, not an optional URL value, because there is no URL type
in the SDK.

### `InputStream` and `OutputStream`: `springboard/sdk/src/stream.rs`

Mirrors `InputStream(data:)`, `OutputStream(toMemory:)`, `OutputStream(toBuffer:capacity:)`, `Stream.Status` and
`hasBytesAvailable` / `hasSpaceAvailable`.

- Status values are `NotOpen`, `Open`, `AtEnd` (input only, once every byte was read) and `Closed`.
- `read` and `write` are synchronous. Before `open()` they fail with `StreamError::NotOpen`; after `close()` with
  `StreamError::Closed`.
- A bounded output stream stores only up to its capacity and then `write` returns 0.
- `OutputStream::written()` returns the bytes stored, as `property(forKey: .dataWrittenToMemoryStreamKey)` does.

Not built: file streams, socket and port streams, `Stream.getBoundStreams` pairs, the `Stream.Event` delegate model and
run-loop scheduling. Those need the system layer, and this round had no scope for them.

## Apple API mirrored

- URLComponents, URLQueryItem (Foundation, URL components)
- InputStream, OutputStream, Stream.Status (Foundation, Streams, Sockets, Ports)

Official pages read with `python3 -I tools/parity/docc.py`: `foundation/urlcomponents`, `foundation/httpcookie`,
`foundation/operationqueue`, and the Foundation landing page for the section list.

## REA evidence

- `ev_a10617f97f1ab67b2eec7033ce17805163cfc69368e3466bbaa1102d3b4bc0d1`: rendered Foundation landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/foundation`, this round).
- `ev_98ed7160e01d4c40fc46c574153ffb427038db487d9965adae2ca830bc3ac639`: the landing-page evidence the matrix already
  held before this round.

## Existing code this round did not change

- `springboard/src/deildu/http.rs` (`Jar`) and `springboard/src/shell/browser/resources.rs` (`CookieJar`) are two
  cookie jars inside the shell. Cookie storage through URLSession is still a gap. Moving the jar into the SDK would
  mean changing the shell transport, which this round left alone.
- The shell's percent-encoder (`springboard/src/shell/browser/nav.rs`, `percent_encode`) is private to the shell. The
  SDK has its own encoder in `url_components.rs`.

## Remaining gaps

- Bonjour: mDNS and DNS-SD discovery and advertising.
- URLSession: WebSocket and stream tasks, cookie storage, credentials and authentication challenges, upload tasks,
  background sessions, disk URLCache.
- URLProtocol and proxies; URLComponents range accessors (`rangeOf*`) and `url(relativeTo:)`.
- OperationQueue, RunLoop and Timer-style scheduling.
- Socket, file and port streams; bound stream pairs; run-loop scheduling of streams and the stream delegate.
- XPC-style service connections.

## Verification

Per the round's rules nothing was built or run. The next stage must build `springboard/sdk` and run its tests:
`url_components` has 7 tests and `stream` has 7 tests, all in `#[cfg(test)] mod tests` in their own file.
`python3 tools/check-names.py` passes on the repo as of this commit.
