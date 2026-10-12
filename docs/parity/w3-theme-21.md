# Wave 3, theme 21: services and peer sessions

Theme: creating XPC services (XPCListener, XPCSession, XPCReceivedMessage), with the connection, endpoint,
extension and Multipeer rows closest to it.

## What was built

Two pure Rust modules in the SDK. Both are std only, with no unsafe code and no hardware or network access.

- `springboard/sdk/src/peer_session.rs`: a `Hub` that routes connections between named listeners and anonymous
  endpoints, and owns the sessions they open. A listener or endpoint takes a `Requirement` (any peer, same user id,
  app id, or both) and rejects peers that fail it. The client calls `connect` and gets a `SessionId`. The service
  takes the same session with `accept` and talks to it as `Side::Server`. A message sent with `wants_reply` must be
  answered once with `reply` before the sender can `take_reply`. A closed session refuses new traffic but still
  delivers what is queued. Bodies are limited to 256 KiB, and listener names to 256 bytes with no control characters.
- `springboard/sdk/src/peer_mesh.rs`: a `Mesh` for one local peer. Other peers join by invitation (`receive_invitation`,
  then `respond` accepts or declines). The session limit counts the local peer, and an acceptance that would reach the
  limit fails with `Full`. `send` takes a list of peers, or every connected peer when the list is empty. A
  `Reliable` send checks every named peer before it delivers anything. An `Unreliable` send skips missing peers and
  reports them as dropped. Payloads are limited to 64 KiB.

Both modules are registered in `springboard/sdk/src/lib.rs` with one `pub mod` line each. The owner name check
(`python3 tools/check-names.py`) exits 0. Each module has a `#[cfg(test)] mod tests` with 8 or more behaviour tests,
and the expected values were worked out by hand from the rules above.

No build or test run was made on this branch, as the task requires.

## Official API names mirrored

Taken from the official pages through `docc.py` and REA. Code uses neutral names, so the Apple names appear here only.

- XPC: `XPCListener`, `XPCSession`, `XPCReceivedMessage`, `XPCEndpoint`, `XPCPeerRequirement`, and the
  "Creating XPC services" and "XPC connections" topics. Official page: https://developer.apple.com/documentation/xpc
- ExtensionFoundation: the `ConnectionHandler` closure, which handles incoming connections. The `AppExtension`
  types are not built. Official page: https://developer.apple.com/documentation/extensionfoundation
- Multipeer Connectivity: `MCSession` (`send(_:toPeers:with:)`, invitations), and the `kMCSessionMaximumNumberOfPeers`
  constant. Official page: https://developer.apple.com/documentation/multipeerconnectivity

The numeric value of the peer-count constant is not in the fetched `docc.py` text. The module therefore takes its
bound from `MAX_SUPPORTED_PEERS` (8), which is this model's own value and is not confirmed against the official page.

## REA evidence

- `ev_58124aa3d3948d8b686b6b7c5ee70b931f427402ba726e4999715c866e8628f5`: XPC landing page (rendered topics and the
  listener, session and received-message types).
- `ev_42102353a79ac67360ddbc8b5df0fe9d81e04fe656647536b3c2024bfdb4057a`: Multipeer Connectivity landing page
  (session phase, invitations, and the `MCSession` topics).
- `ev_6c9fbf5728ddc4cdd7ffeca9800ae3f4052acbd3ef557cb821e0a9d9766de8c7`: ExtensionFoundation landing page
  (`ConnectionHandler`, which handles incoming XPC connections).

## Matrix rows closed

Each row is partial, not parity. Every "missing" note in the matrix still holds for the parts left out.

| Framework slug | Feature (exactly as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| xpc | Creating XPC services (XPCListener, XPCSession, XPCReceivedMessage) | partial | springboard/sdk/src/peer_session.rs |
| xpc | XPC connections and endpoints (XPCEndpoint, anonymous listeners) | partial | springboard/sdk/src/peer_session.rs |
| extensionfoundation | Building an extension (AppExtension, AppExtensionConfiguration, ConnectionHandler) | partial | springboard/sdk/src/peer_session.rs (Listener accept, handler side only) |
| multipeer-connectivity | Sessions and invitations (MCSession, invite and accept) | partial | springboard/sdk/src/peer_mesh.rs |
| multipeer-connectivity | Message-based data (send Data to peers) | partial | springboard/sdk/src/peer_mesh.rs |
| multipeer-connectivity | Session limits (kMCSessionMaximumNumberOfPeers) | partial | springboard/sdk/src/peer_mesh.rs |

## Still missing

- No process boundary. The hub lives in one process, so an endpoint cannot be sent to another process. Real
  cross-process delivery needs the shell to carry the messages.
- No typed XPC object model. Bodies are bytes, and the model does not encode XPC-style dictionaries.
- No code-signing or entitlement check. `Peer` carries a user id and an app id only.
- No on-demand launch or idle shutdown for services, which the service supervisor owns.
- No discovery, advertising, streams or file resources for Multipeer. The `Mesh` is a model of the session only.
- Multipeer has no encryption preference and no background disconnect.
