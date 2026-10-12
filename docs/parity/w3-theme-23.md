# Wave 3, theme 23: DNS-SD domains, records and object identifiers

Theme: "Enumerate recommended domains (DNSServiceEnumerateDomains)", with the DNS-SD rows that sit
next to it and the GSS object-identifier row.

## What was built

Two std-only modules in `springboard/core` (pure models, no socket, no daemon, no query is sent):

- `springboard/core/src/dnssd.rs`
  - `DomainEnumeration`: applies enumeration replies (`apply(flags, domain)`) and keeps the browse and
    registration lists in insertion order. `Add` adds or updates, its absence removes. `Default` marks the
    default of its kind and clears the previous default. `MoreComing` holds `is_complete()` false until the
    last reply.
  - `DnsServiceError`: the 37 error names the reference lists, with `suffix()` and `Display` as `kDNSServiceErr_*`.
  - Flags `FLAG_MORE_COMING`, `FLAG_ADD`, `FLAG_DEFAULT`, `FLAG_BROWSE_DOMAINS`, `FLAG_REGISTRATION_DOMAINS`
    and the interface indices `INTERFACE_ANY`, `INTERFACE_LOCAL_ONLY`, `INTERFACE_UNICAST`.
  - DNS wire format (RFC 1035 layout, shared with multicast DNS): `Message::encode` and `Message::decode`
    with label encoding, compression-pointer reads (loop-limited), and `Record`/`Question`. Decoded rdata
    for NS, PTR, CNAME and SRV is made self-contained, so compressed names inside it are expanded.
  - `query(id, name, rtype)` builds a one-question query. `answers_for` filters answers.
  - Record data: `address_rdata`/`parse_address` (A, AAAA), `name_rdata`/`parse_name_rdata` (PTR, CNAME, NS),
    `srv_rdata`/`parse_srv`, `txt_rdata`/`parse_txt` (reuses `txt_record::TxtRecord`).
  - `resolve_addresses(records, name)`: follows CNAME records (eight hops at most) to an A or AAAA answer.
  - `same_name`: case-insensitive, ignores a trailing dot.
- `springboard/core/src/oid.rs`
  - `Oid`: parse dotted form (`Oid::parse`), arcs, `Display`, DER content and full DER (`to_der`,
    `from_der`, `from_der_content`), with base-128 arcs and the first-two-arcs rule.
  - `OidSet`: duplicate-free set in insertion order.

Both are registered with one `pub mod` line each in `springboard/core/src/lib.rs`.

## Official API names mirrored

- `dnssd` landing page (DNS Service Discovery C): `DNSServiceEnumerateDomains`, `DNSServiceDomainEnumReply`,
  `DNSServiceQueryRecord`, `DNSServiceGetAddrInfo`, `DNSServiceAddRecord`, `DNSServiceUpdateRecord`,
  `DNSServiceRemoveRecord`, `DNSServiceRegisterRecord`, `DNSServiceErrorType`, `DNSServiceFlags`,
  `DNSServiceProtocol`, the `kDNSServiceErr_*` constants and `kDNSServiceClass_IN`.
  - `DNSServiceFlags` is a UInt32 type alias (DocC type declaration page).
  - `kDNSServiceErr_*` names: the dnssd Constants list on the landing page.
- `GSS` landing page and "Object Identifiers" collection group: `gss_OID`, `gss_OID_set`, `gss_oid_equal`,
  `gss_oid_to_str`, `gss_create_empty_oid_set`.

## Evidence

- REA `ev_533ce18f32b88a591102b1169420bb53f634c1fddb3ac8d97f4387fcc8212e0d`: `https://developer.apple.com/documentation/dnssd` (landing page, lists the constants and the functions).
- REA `ev_7cc58e99c0d61b6e4f891de9a5108c790523ca1dda654a48b0d149161e61fb70`: `https://developer.apple.com/documentation/dnssd/dnsserviceflags` (UInt32 type alias).
- REA `ev_f3ca4ee4a8536ecdd3979cb211c2a3c1f4b8051dc5a6bbba48fcb2f1a61a8e5f`: `https://developer.apple.com/documentation/gss` (Object Identifiers group).

Caveats recorded for the reviewer:

- DocC symbol pages for `DNSServiceEnumerateDomains`, `DNSServiceQueryRecord`, `DNSServiceGetAddrInfo`,
  `kDNSServiceInterfaceIndexAny` returned 404 through `tools/parity/docc.py`. The REA landing page is the
  evidence for their names.
- The numeric values of the flags (`MoreComing` 0x1, `Add` 0x2, `Default` 0x4, `BrowseDomains` 0x40,
  `RegistrationDomains` 0x80) and of the interface indices (`Any` 0, `LocalOnly` all ones, `Unicast` all ones
  less one) are not on the DocC pages that were fetched. They follow the DNS-SD header as known to the
  author and need a check against the official header before release. The numeric error codes are not
  modelled, only the names.
- Nothing here sends or receives a packet. The `mDNS` port 5353 row is closed only at the wire-format level.

## Matrix rows closed

Status is `partial` for every row. The rows are not `parity`: the daemon connection, the NAT mapping, the
socket and the app-facing API are still missing.

| framework slug | feature (as in data json) | status | BifrOSt path |
|---|---|---|---|
| dnssd | Enumerate recommended domains (DNSServiceEnumerateDomains) | partial | springboard/core/src/dnssd.rs (DomainEnumeration) |
| dnssd | Resolve hostnames to IPv4 and IPv6 (DNSServiceGetAddrInfo) | partial | springboard/core/src/dnssd.rs (resolve_addresses, parse_address) |
| dnssd | Custom records on a service (AddRecord, UpdateRecord, RemoveRecord, RegisterRecord) | partial | springboard/core/src/dnssd.rs (Record, srv_rdata, txt_rdata, name_rdata, address_rdata) |
| dnssd | Query arbitrary DNS records (DNSServiceQueryRecord) | partial | springboard/core/src/dnssd.rs (query, answers_for, Message::decode) |
| dnssd | Error codes, flags and interface indices (kDNSServiceErr_*, kDNSServiceFlags*, kDNSServiceInterfaceIndex*) | partial | springboard/core/src/dnssd.rs (DnsServiceError, FLAG_*, INTERFACE_*) |
| dnssd | Multicast DNS and DNS-SD wire protocol on port 5353 | partial | springboard/core/src/dnssd.rs (Message::encode, Message::decode) |
| gss | Object identifiers (gss_OID, gss_OID_set) | partial | springboard/core/src/oid.rs (Oid, OidSet) |

Not closed by this theme: "Connection and reference lifecycle" (dnssd), "NAT port mapping" (dnssd),
and the rest of the theme's rows (CFNetwork, Network Extension, Foundation, System Configuration, Disk
Arbitration, GSS name handling and tokens).

## Tests

`cargo` was not run, per the task. The `#[cfg(test)]` modules hold 15 tests in `dnssd.rs` and 6 in `oid.rs`,
with expected values worked by hand from the encoding rules (for example `1.2.840.113549` encodes to
`2A 86 48 86 F7 0D`, and the name `_http._tcp.local.` takes 18 wire bytes).
