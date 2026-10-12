# Wave 4, theme 12: DispatchData and CSV writing

Theme 12 spans ten frameworks (Dispatch, GSS, IOSurface, Matter, dnssd, Force Feedback, Endpoint Security,
Compression, Combine, Tabular Data). Most of its gap rows need hardware, a daemon or a kernel service
(GSS contexts, Matter accessories, dnssd sockets, force-feedback devices, Endpoint Security clients), so this round
builds the two pure-Rust pieces that the theme can honestly carry: the dispatch data buffer and the CSV writer.

## What was built

- `springboard/sdk/src/dispatch_data.rs`: `DispatchData`, an immutable byte sequence made of discontiguous regions.
  Regions share their storage through `Arc`, so `append(&DispatchData)` and `subdata` copy no bytes. It provides
  `empty`, `from_slice`, `from_vec`, `append`, `append_slice`, `get`, `subdata`, `region`, `copy_bytes`,
  `enumerate_bytes` (stop flag as a `bool` return) and `to_vec`, with content equality that ignores region boundaries.
  Out-of-range access returns `None` or clamps, where the framework traps.
- `springboard/sdk/src/csv_writer.rs`: `CsvWriter` (RFC 4180 quoting, a configurable delimiter and line ending, header
  names kept for error reports) and `CsvWritingError::BadEncoding { row, column, data }`, raised for a byte field that
  is not valid UTF-8. The write stops at that field and writes nothing of the failing record.

Both modules are registered in `springboard/sdk/src/lib.rs` with one `pub mod` line each. Neither uses a new crate,
`unsafe`, hardware or network access.

## Official API names mirrored

- Dispatch: `DispatchData`, `DispatchData.empty`, `init(bytes:)`, `append(_:)`, `subdata(in:)`, `region(location:)`,
  `copyBytes(to:from:)`, `enumerateBytes(block:)`, `subscript`.
  Official page: https://developer.apple.com/documentation/dispatch/dispatchdata
- TabularData: `CSVWritingError`, `CSVWritingError.badEncoding(row:column:_:)`, `CSVWritingError.row`,
  `CSVWritingError.column`. Official page: https://developer.apple.com/documentation/tabulardata/csvwritingerror
  The writer type itself (the call that produces the CSV text) is not named on that page, so its method names are
  BifrOSt's own.

## REA evidence

- `ev_864e6d6e7c238bea8ed6de5e2a5d013fd8746b9bfb7b8cc79a31caa6626d6eab` (DispatchData, rendered official page)
- `ev_3efb62dee15580a43d5a1b99d239d1287e77c0028ab28184e6a94ab7191aa22b` (CSVWritingError, rendered official page)

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| dispatch | DispatchData | partial | springboard/sdk/src/dispatch_data.rs |
| tabular-data | CSV writing | partial | springboard/sdk/src/csv_writer.rs |

Status is `partial`, not `parity`: the writer covers the write side only, with no column-typed encoding, no
`DataFrame` input, and no `CSVWritingError` for a delimiter that cannot be written. DispatchData has no
`DispatchIO` integration and no `Dispatch.Data` bridge to the file-reading path.

## Rows left open in this theme

IOSurface (use counts, attribute dictionary), GSS, Matter, dnssd, Force Feedback, Endpoint Security, Compression
stream filters and Combine async publishers need device, daemon or kernel services, or an official page that was not
fetched in this round. They stay as gaps.

## Verification

Tests were written by hand from the rules above. They were not built or run in this round, per the wave brief.
