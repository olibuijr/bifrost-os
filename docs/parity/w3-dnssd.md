# Wave 3: DNS-SD (dnssd) parity

Two gap rows of the `dnssd` framework now have pure Rust models in `springboard/core/src`. Both are
std-only and open no socket. Nothing is wired into the shell or the apps yet, and nothing has been
built or run on the host, the VM or Midget.

## What was built

- `springboard/core/src/dns_message.rs`: DNS message wire format, encode and decode. Covers the
  RFC 1035 header, questions and resource records, the mDNS class bits from RFC 6762 (cache-flush
  on answers, unicast-response on questions), and the PTR, SRV, TXT, A and AAAA record data from
  RFC 6763. Names are written without compression and read with compression pointers. A pointer
  must point backwards, and pointer chains are capped. Other record types are kept as raw bytes.
  Tests: 9 behaviour tests, with the expected bytes worked out by hand.
- `springboard/core/src/dns_record_set.rs`: a record set for one service instance. `RecordSet` adds,
  updates and removes records by handle, as the add, update and remove calls work on a registered
  service. `service_records` builds the RFC 6763 set for one instance: a PTR from the service type,
  SRV and TXT for the instance, and an A record for the host. Tests: 6 behaviour tests.

Both modules are declared in `springboard/core/src/lib.rs` with one `pub mod` line each.

## Apple names mirrored

From the official `dnssd` pages (`https://developer.apple.com/documentation/dnssd`):

- `DNSServiceAddRecord`, `DNSServiceUpdateRecord`, `DNSServiceRemoveRecord`: `RecordSet::add`,
  `RecordSet::update`, `RecordSet::remove`. The official DocC page gives these as record-set
  operations on a registered service, which is the behaviour mirrored.
- `DNSServiceRegister` record set: `service_records` returns the set a service instance publishes.
  It does not register anything.

The DocC pages give constant names only, with no numeric values, so the error-code, flag and
interface-index rows were not closed. Their numeric values are not in the official pages that were
read, so they were not invented.

## Not covered

- No socket, no mDNS responder or querier on port 5353, and no DNSServiceRef or dispatch integration.
- `DNSServiceRegisterRecord` (an individual record on a connection) is not modelled.
- Query, address lookup, browse and resolve remain gaps, as do NAT port mapping and domain enumeration.

## REA evidence

- `ev_93ec879d0681429b2aeb9e6400069a8e41d7ba01ba98f2e030eec0799c5042ee`: rendered
  `https://developer.apple.com/documentation/dnssd` (landing page, constant list).
- `ev_652bd44970aeadd7f824944f537895812cb6eb118df514b0ce7fa9316e59ce84`: the framework's
  existing evidence in `docs/parity-matrix/data/dnssd.json`.

## Matrix rows closed

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| dnssd | Multicast DNS and DNS-SD wire protocol on port 5353 | partial | springboard/core/src/dns_message.rs |
| dnssd | Custom records on a service (AddRecord, UpdateRecord, RemoveRecord, RegisterRecord) | partial | springboard/core/src/dns_record_set.rs |

The custom-records row stays partial because `RegisterRecord` is not modelled and nothing is sent.
The wire-protocol row stays partial because there is no socket.
