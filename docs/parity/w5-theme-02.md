# Wave 5, theme 02: local push connectivity

Branch `parity/w5-theme-02`, based on `parity/integration` (wave 4, 8200cb0f).

## What was built

Two small pure modules, std only, with no socket and no unsafe code. Nothing is wired into the shell or the SDK yet.

- `springboard/core/src/natpmp.rs` (new). NAT port mapping over NAT-PMP (RFC 6886). It encodes the external address request, the map request (UDP or TCP, with a lifetime, where zero deletes) and the delete request. It decodes replies, reports gateway refusals by result code, builds a `Lease` from a granted mapping (renew at half the lifetime, which is this module's choice and not the RFC's), and detects a gateway restart from the epoch. The caller sends the bytes to port 5351 and passes back what it receives.
- `springboard/core/src/notify_center.rs` (extended). Signal-token delivery. `register_signal(name, signal)` takes a signal number from 1 to 64 (`MAX_SIGNAL`). Each post to the name queues that signal, and `take_signals()` drains the queue in post order. The caller raises the signals itself. No kernel signal path is used, so the delivery is modelled, not real.

Module names were checked against `tools/parity/parity.py search`. There was no existing NAT-PMP, PCP or UPnP code. The notify center already existed and was extended rather than duplicated.

## Official API names mirrored

- `DNSServiceNATPortMappingCreate`, `DNSServiceNATPortMappingReply` (dnssd). The module provides the wire-level request and reply model that such a call would sit on. It does not provide the call-level API.
- `notify_register_signal(_:_:_:)` (Darwin Notify). The module provides the name-to-signal registration and queue. It does not provide a token-keyed API that matches the call's signature.

## REA evidence

- `ev_dd2213cc67a2d7376c19e5a7e6bf3391c797c8e0d82378285c7132b4e9bf7f20`: the dnssd framework page (`https://developer.apple.com/documentation/dnssd`). It was rendered and read. This is the evidence for the dnssd row.
- `ev_4ffc1abdea742ef66733b30bca1fff93a9b3e3951a05e46f45a856fcf6a8e16b`: `https://developer.apple.com/documentation/dnssd/dnsservicenatportmappingcreate`. The page returned "The page you're looking for can't be found", so it is not used as a source of facts.
- `ev_ef0d4dcb8b2952ba1ed796b0d99653b652e375b1edd45ccd08a9f9b411680794`: `https://developer.apple.com/documentation/system/notify_register_signal`. The page also returned "can't be found", so it is not used as a source of facts.
- `docc.py` returned HTTP 404 for `documentation/dnssd` and `documentation/darwin_notify`.

The two signal and NAT behaviours above come from the matrix rows' own wording and from RFC 6886, not from the two missing pages.

## Matrix rows closed (new status partial)

| Framework slug | Feature (as written in the matrix) | Status | BifrOSt path |
|---|---|---|---|
| dnssd | NAT port mapping (DNSServiceNATPortMappingCreate) | partial | springboard/core/src/natpmp.rs |
| darwin-notify | Signal-based delivery (notify_register_signal) | partial | springboard/core/src/notify_center.rs |

## Not closed

The other rows in theme 02 remain gaps: local push connectivity, APNs, background push, web push, Core Location authorization, CallKit, Bonjour (NetService), CFHost resolution and diagnostics, and the network extension rows. Each needs a daemon, an entitlement or a hardware path that a pure model cannot supply.

## Verification

No builds or tests were run, as the wave rules require. The tests are written with hand-worked expected values: 8 in `natpmp.rs` and 5 new in `notify_center.rs`. `python3 tools/check-names.py` exits 0.
