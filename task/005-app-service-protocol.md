# 005 — Phase 3: version and harden app/service communication

Status: planned. Depends on: 003. Parent: `001`.

## Problem (verified)

- `springboard/proto/src/lib.rs:10`: `SDK_VERSION: u32 = 1`. `shell/apphost.rs:340` rejects any Hello whose version differs or whose app ID does not match. There is no negotiation.
- `ToShell` (`proto/src/lib.rs:78-93`) has no request IDs. `OpenUrl`, `Notify`, `SetClipboard`, `Haptic`, and `WantsKeyboard` are fire-and-forget (`sdk/src/runtime.rs:38-50`). The `Ctx` in `sdk/src/runtime.rs:23` has no correlated result.
- Limits in place: `MAX_PACKET` 16 KiB (`proto/src/lib.rs:15`); `MAX_FRAME` 256 KiB (`proto/src/svc.rs:12`); `MAX_FDS` 4 (`proto/src/sys.rs:138`); at most 128 control nodes, and decode rejects more (`proto/src/lib.rs:339`); context menu items capped at 32 on encode.
- `ToApp` and `ToShell` decode paths reject unknown tags (`DecodeError::UnknownTag`). Keep that.

## Steps

1. Introduce a negotiated range. Hello carries `min_version` and `max_version` plus a feature bitmask. The host answers with the chosen version and features. A v1 Hello (no range) is accepted and treated as version 1 with no new features.
2. Send new messages only to clients that negotiated them. Specify the error for an incompatible version and the supported compatibility window in the protocol doc.
3. Add request IDs and typed results for the operations that need acknowledgement: `OpenUrl`, `Notify`, `SetClipboard`, and a broker call if one exists. Results carry `Ok`, `Denied`, `Unsupported`, `Timeout`, `Cancelled`, or `Disconnected`. Do not add generic scaffolding for unused operations.
4. Add deadlines and cancellation. Disconnect completes every pending request with `Disconnected`. A late result for a finished or unknown ID is dropped and counted.
5. Bound queue length, in-flight requests, string lengths, packet sizes, descriptor counts, geometry, and buffer allocations. Define backpressure (reject new work with `Busy`) and a rate limit for notifications and URL opens. Never block the render loop on broker work.
6. Derive app identity and permissions from the authenticated launch and channel, never from caller-supplied fields. Handle replayed IDs, duplicate IDs, unknown messages, unexpected descriptors, and reconnect generations. Validate shared-buffer sizes and lifetimes and release ownership on disconnect.
7. Give apps an asynchronous completion mechanism that fits the existing event loop: a result callback delivered as an event. Do not add an async runtime unless a measured need appears.
8. Document the protocol in `springboard/docs/apps.md`.

## Acceptance

- The current v1 fixture app works unchanged with the new host.
- A negotiated client receives typed completion for each request type.
- Cancellation and disconnect finish pending requests.
- Malformed or unbounded input cannot grow memory without limit or acquire authority.
- Cross-version fixtures (v1 client against new host; new client against a v1-only host where applicable).
- Decode property or fuzz tests for all new variants.
- Service-restart tests for the audio client path (`svc.rs`).
- Checks: focused `bifrost-proto` and `bifrost-app` tests; full run belongs to the batch acceptance.

## Compatibility

Wire format is additive. v1 clients keep working. The SDK version constant changes only with a documented migration note.
