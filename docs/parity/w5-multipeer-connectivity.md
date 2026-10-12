# Wave 5: Multipeer Connectivity (nearby peers)

Official reference: https://developer.apple.com/documentation/multipeerconnectivity (the framework is marked deprecated
on the rendered pages, with "Use Network Framework instead"; the pages were read for their API names and rules only).

## What was built

Three pure Rust additions in `springboard/sdk/src/`. Nothing is wired into the shell or into apps, and nothing has been
built or run (wave rules: no builds in this round).

- `peer_identity.rs` (new): `PeerId`, a peer with a read-only display name and a number id. Mirrors `MCPeerID`:
  `PeerId::new(display_name)` (a new id per call, even for the same name), `display_name()`, and a text record
  (`to_record` / `from_record`) so an app can keep one identity over time, as the official page recommends for a stable
  peer ID. The name is at most 63 bytes of UTF-8, not empty, and without control characters, as the official page states.
- `nearby_service.rs` (new): `ServiceType`, the check an advertiser applies to its service type before publishing. The
  official `init(peer:discoveryInfo:serviceType:)` page says only that an invalid value is refused for not being "a legal
  Bonjour service type". The rules used (1 to 15 bytes, lowercase letters, digits and hyphens, at least one letter, no
  edge or doubled hyphen) follow the Bonjour service-name rules in RFC 6335 section 5.1, which is not Apple
  documentation. Uppercase is refused here, which RFC 6335 allows; this keeps one spelling per name.
- `peer_mesh.rs` (extended): `Code`, the error categories an app switches on, with the platform's names
  (`cancelled`, `invalidParameter`, `notConnected`, `timedOut`, `unavailable`, `unknown`, `unsupported`), and
  `Error::code()` mapping the mesh's own errors to them. Bad names, limits, invitation ids, payload size and a repeated
  invitation from a connected peer map to `invalidParameter`; a full session maps to `unavailable`; a reliable send to a
  missing peer maps to `notConnected`. The mesh never returns `cancelled`, `timedOut`, `unknown` or `unsupported`.
  One test (`errors_map_to_the_platform_codes_by_cause`) was added to the module's existing tests.

`springboard/sdk/src/lib.rs` gains two `pub mod` lines (`peer_identity`, `nearby_service`).

## Official API names mirrored

| Official name | BifrOSt |
|---|---|
| `MCPeerID`, `init(displayName:)`, `displayName` | `peer_identity::PeerId`, `new`, `display_name` |
| `MCNearbyServiceAdvertiser` `serviceType` (validated at init) | `nearby_service::ServiceType::new` |
| `MCError.Code` cases (`invalidParameter`, `notConnected`, `unavailable`, ...) | `peer_mesh::Code` and `Code::name` |
| `MCError` (error domain) | not mirrored: `Code` covers the codes only, no domain string |

## REA evidence

- `ev_fa8d23b5107d377d75165b66b061db14a5b2e5ba9c23ac138952ec9979cab74d`: MCError type page (error code list; deprecated).
- `ev_5ac1f7e6f5321bf103460881e5b38a15483b925cdc4f46906bd34ba1f0c7aac0`: MCPeerID class page (63-byte UTF-8 name limit,
  unique id per init, archive for a stable id; deprecated).
- `ev_320ee65cca13961f9dfb5c86bd32b2d4cd915effe05d499556fcbdb2fee2119c`: MCNearbyServiceAdvertiser init page
  ("not a legal Bonjour service type"; deprecated).

## Matrix rows closed (for the lead to update centrally)

| Framework slug | Feature (as written in the matrix) | Previous status | New status | BifrOSt path |
|---|---|---|---|---|
| multipeer-connectivity | Peer identity (MCPeerID) | partial | partial | springboard/sdk/src/peer_identity.rs |
| multipeer-connectivity | Error reporting (MCError, MCErrorDomain) | partial | partial | springboard/sdk/src/peer_mesh.rs (Code, Error::code) |
| multipeer-connectivity | Advertising a service (MCNearbyServiceAdvertiser) | os | partial | springboard/sdk/src/nearby_service.rs (ServiceType) |

Still open in this framework: browsing and advertising over the shell's beacon (the app-facing API that would use
`ServiceType`), the advertiser and browser UI, encryption preference (the transport is plaintext), and streaming.
