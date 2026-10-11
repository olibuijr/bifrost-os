# Theme 19: connection and reference lifecycle (parity/theme-19)

Scope: the DNS-SD connection, TXT, error and record gaps in `docs/parity-matrix/themes.json`
(theme-19 has 23 rows across dnssd, GSS, CFNetwork, IOSurface and Disk Arbitration). This branch closes the
part that can run as pure Rust with no network service and no socket: the TXT record data model. The
other rows stay open, and the reasons are listed below.

## What was built

- `springboard/core/src/txt_record.rs` (new module, one `pub mod txt_record;` line in `springboard/core/src/lib.rs`).
  - `TxtRecord`: an ordered record of unique keys. `set` (replaces in place), `get` and `contains_key`
    returning `Presence`, `remove`, `item_at`, `len`, `encoded_len`.
  - `Presence`: `Absent`, `NoValue` (`key`), `Empty` (`key=`), `Value(&[u8])` (`key=value`).
  - `to_bytes` and `from_bytes`: the length-prefixed string encoding of the record data.
  - `TxtError`: `InvalidKey`, `TooLong` (string over 255 bytes), `StorageFull` (record over 65535 bytes),
    `Truncated`.
  - 7 behaviour tests: the four key states, invalid keys, in-place replacement, the 255-byte and 65535-byte limits,
    the exact encoded layout, round trip, truncation and first-duplicate rule, and remove and index order.

## Official API names mirrored

Official documentation (rendered pages read with `python3 -I tools/parity/docc.py` and the REA page capture):

- `TXTRecordRef` (symbol page, `https://developer.apple.com/documentation/dnssd/txtrecordref`): opaque record
  type. The module mirrors its behaviour, not its opaque pointer and storage.
- `TXTRecordSetValue(_:_:_:_:)`: adds a key with an optional value. Key rules: printable ASCII 0x20 to 0x7E,
  no `=`, 9 characters or fewer recommended. Replaces an existing key. Four key states: absent, present with
  no value, present with empty value, present with value.
- `TXTRecordGetValuePtr(_:_:_:_:)`: returns NULL for an absent key or a key with no value, and a value of
  length zero for an empty value. `TXTRecordContainsKey` distinguishes the two NULL cases. The module keeps
  this split: `Absent` and `NoValue` are distinct.
- `TXTRecordGetItemAtIndex`, `TXTRecordRemoveValue`, `TXTRecordGetBytesPtr`, `TXTRecordGetLength`: mirrored as
  `item_at`, `remove`, `to_bytes` and `encoded_len`.
- `TXTRecordCreate(_:_:_:)`: its page path returned 404 in `docc.py`
  (`dnssd/txtrecordcreate(_:)`), so it is not cited here. The module needs no creation call.
- The Apple error result for a bad key (`kDNSServiceErr_Invalid`) and storage overflow
  (`kDNSServiceErr_NoMemory`) map to `TxtError::InvalidKey` and `TxtError::StorageFull`.

Choices not stated in the official page read this session, documented here:
- A zero-length string in parsed data is skipped.
- When a key repeats in parsed data, the first occurrence wins.
- The 65535-byte cap follows from the 16-bit length of a TXT record; the official page does not state the cap.
- An empty key is refused (the official rule says printable characters, and an empty key is not one).

## REA evidence

- `ev_1fe2260bc21de40d3cd2acf67bd965d2cb15aa7604e5ec790677fe32702067de`: the dnssd framework landing page
  (`https://developer.apple.com/documentation/dnssd`), with the `kDNSServiceErr_*` and flag constants in its
  variable list.
- `ev_6d37c4339dec4b04a3e1516e28332b07678e624a99701b2f6fcca82625f2c521`: `TXTRecordSetValue(_:_:_:_:)`
  (`https://developer.apple.com/documentation/dnssd/txtrecordsetvalue(_:_:_:_:)`), with the key rules and the
  four key states.

## Matrix rows closed

| Framework slug | Feature (as written in `docs/parity-matrix/data/dnssd.json`) | New status | BifrOSt path |
|---|---|---|---|
| dnssd | TXT record construction and parsing (TXTRecordCreate, TXTRecordSetValue, TXTRecordGetValuePtr and the rest) | partial | `springboard/core/src/txt_record.rs` |

Partial, not parity: the module is a core system utility. It has no SDK surface for apps, and no DNS-SD
resolve or browse call feeds it yet. The matrix's parity rule needs an app-facing API.

## Rows left open, and why

- dnssd connection and reference lifecycle, record add, update and remove, query, address lookup, enumerate
  domains, NAT port mapping, and the multicast wire protocol: these need sockets, a mDNS responder or a daemon
  connection. The task rules forbid network services and sockets.
- dnssd error codes and flags (`kDNSServiceErr_*`, `kDNSServiceFlags*`): the official pages list the constant
  names, but this session did not read their numeric values from a source that could be cited. Left open rather
  than guessed.
- GSS (context services, Kerberos, credentials, status codes, OIDs, buffers, message protection, mechanisms,
  tokens) and Disk Arbitration and CFNetwork host resolution and IOSurface global lookup: not started in this
  round. Each needs its own official-page read and a design, which exceeds one theme pass with a single module.

## Tooling notes

- The akurai index `code_search` timed out on both the hybrid and lexical queries (`error: timeout`). The
  check of existing code used the directory listing of `springboard/core/src` and `springboard/ui/src`, which
  showed no DNS-SD, GSS, IOSurface or disk module and no existing TXT module. The name `txt_record` was checked
  against that listing.
- `python3 tools/parity/parity.py search` fails on the local embedding file (`EOFError: No data left in file`
  in `tools/parity/semantic.py`). The matrix rows were read from `docs/parity-matrix/data/*.json` and
  `themes.json` directly.
- `docc.py` returned 404 for `dnssd/txtrecordcreate(_:)`. The paths that worked use the parenthesised
  symbol form (for example `dnssd/txtrecordsetvalue(_:_:_:_:)`).
- No cargo, rustc or test run was done, per the task rules. The module is unbuilt.
- `python3 tools/check-names.py`: exit 0, no hits.
