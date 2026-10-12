# Wave 5: CFNetwork gap rows (host lookup, service names, diagnosis)

Branch `parity/w5-cfnetwork`, started from `parity/integration` (waves 1 to 4). Three pure std modules in the
app SDK close three CFNetwork rows that were gaps. Nothing here sends a query, opens a socket or reads the
network. The code is not built or tested yet (wave rule: no builds in coding waves; the integration build runs
after the waves merge).

## What was built

| Module | What it does | Official API it mirrors |
|---|---|---|
| `springboard/sdk/src/host_addressing.rs` | Normalizes a host name, reads numeric addresses, orders and filters answers by family (IPv4 first, no repeats), and runs one lookup through `Idle`, `Resolving`, `Resolved`, `Failed`. A numeric address resolves at once with no query. The caller supplies the answer through `finish` or `fail`. | CFHost, CFHostCreateWithName, CFHostCreateWithAddress, CFHostStartInfoResolution, CFHostGetAddressing, CFHostCancelInfoResolution (https://developer.apple.com/documentation/cfnetwork/hosts) |
| `springboard/sdk/src/dns_service.rs` | Service type names (`_http._tcp`), the full name in a domain (`_http._tcp.local.`), instance names with escaped dots, and TXT records in the length-prefixed `key=value` wire format (encode, decode, first key wins, flag keys with no value). It sends and receives nothing. | CFNetService, CFNetServiceCreate, CFNetServiceCreateTXTDataWithDictionary, CFNetServiceCreateDictionaryWithTXTData, CFNetServiceGetType, CFNetServiceGetDomain, CFNetServiceGetName, CFNetServiceGetPortNumber, CFNetServiceSetTXTData, CFNetServiceGetTXTData (https://developer.apple.com/documentation/cfnetwork/network-services) |
| `springboard/sdk/src/net_diagnostic.rs` | Takes an observation of each stage (link, address, name, connection, security, status) and returns the first failing stage with a title and the next step. A status of 500 or more counts as a server failure. | CFNetDiagnostic, CFNetDiagnosticCreateWithURL, CFNetDiagnosticCreateWithStreams, CFNetDiagnosticDiagnoseProblemInteractively, CFNetDiagnosticStatusValues (https://developer.apple.com/documentation/cfnetwork/network-diagnostics) |

Each module is registered with one `pub mod` line in `springboard/sdk/src/lib.rs`. Each has a
`#[cfg(test)] mod tests` block with behaviour tests whose expected values were worked out by hand from the rules
in the code (for example the TXT bytes for `txtvers=1` and `path=/x` are `9 txtvers=1 7 path=/x`, and a name of
four 63-byte labels is 255 bytes, over the 253-byte limit).

Not built, and why: the network service browser and monitor (multicast transport), the proxy auto-configuration
script, FTP, and the shell-side resolver hookup. These need a network service or a socket, which the rules keep out
of this wave. The diagnosis model takes its observations from the caller and does not probe the network itself.

## Official API and evidence

- Official landing page, rendered: https://developer.apple.com/documentation/cfnetwork (REA evidence id
  `ev_bf0e265a1220c1c22cc4b618089fc2143dfc5884fd1cd16e07d0c1a06dfb2471`). The rendered page lists the Hosts, Network
  Services and Network Diagnostics sections used here.
- DocC: `python3 -I tools/parity/docc.py cfnetwork` (sections "Hosts", "Network Diagnostics", "Network Services").
  The per-section and per-symbol DocC paths returned 404, so the landing page is the cited source. The symbol
  names above come from that page.

## Matrix rows closed

Status moves from `gap` to `partial`. Parity needs the feature working on the VM and Midget, which has not happened,
so no row is `parity` or `os`.

| Framework | Feature (as in `docs/parity-matrix/data/cfnetwork.json`) | New status | BifrOSt path |
|---|---|---|---|
| CFNetwork | Host name resolution | partial | springboard/sdk/src/host_addressing.rs |
| CFNetwork | Network services (Bonjour) | partial | springboard/sdk/src/dns_service.rs |
| CFNetwork | Network diagnostics | partial | springboard/sdk/src/net_diagnostic.rs |

Still open in the matrix after this wave: the host lookup has no network query behind it yet (the resolver in
springboard/src/deildu/dns.rs is internal), there is no browse or register transport, and the diagnosis is not
connected to a request. The matrix rows are not edited here; the lead updates them centrally.
