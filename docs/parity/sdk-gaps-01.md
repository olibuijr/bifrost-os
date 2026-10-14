# Parity sdk-gaps-01: models for gap rows

Wave w7. Eleven gap rows were assessed. Seven now have a built model and are `partial`, and four stay `gap` with the
blocker written in the row note.

## What was added or found

| Row | Module | Tests |
|---|---|---|
| Hybrid public key encryption (apple-cryptokit) | `springboard/ui/src/hpke.rs` (existing) | `springboard/ui/tests/parity_sdk-gaps-01.rs` |
| Live broadcast controller and handlers (replaykit) | `springboard/ui/src/broadcast.rs` (new) | same file |
| Broadcast picker UI (replaykit) | `springboard/ui/src/broadcast.rs` (new) | same file |
| Providers and content items (media-player) | `springboard/ui/src/content_items.rs` (new) | same file |
| Privacy attribution and local network identity (network) | `springboard/ui/src/network_attribution.rs` (existing) | same file |
| Mail handoff (mailkit) | `springboard/ui/src/compose_handoff.rs` (existing) | same file |
| Ethernet bond and VLAN plans (system-configuration) | `springboard/core/src/vlan_plan.rs` (existing) | `springboard/core/tests/parity_sdk-gaps-01.rs` |

## Not built

- Post-quantum KEM and signatures: needs the FIPS 203 and FIPS 204 arithmetic and the standard's known-answer vectors.
- Virtual camera providers: needs a loopback capture device and a way to publish it to other apps.
- Quartz composition runtime: needs a node-graph runtime that is not in this repository.
- Cloud backup of game saves: the backend is in `bifrost-cloud`, and no PC client in this repository calls it.

## Lab machine

None of the new tests has run yet. The lead builds and runs `cargo test -p bifrost-ui --test parity_sdk-gaps-01` and
`cargo test -p bifrost-core --test parity_sdk-gaps-01` on the lab machine. Nothing in these models reaches the shell,
the network service or a device, so no Midget check is needed before the rows are `partial`.
