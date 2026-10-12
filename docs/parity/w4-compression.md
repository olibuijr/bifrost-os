# Wave 4: Compression (app-facing SDK API)

## What was built

`springboard/sdk/src/compression.rs` (`bifrost_app::compression`, pure Rust, std only) gives apps the
buffer, stream and filter shapes of the reference Compression API over the codecs the system layer already
has: DEFLATE (`springboard::files::zip::deflate`, greedy LZ77 with one fixed-Huffman block) and the zlib
wrapper (`springboard::shell::books::engine::inflate`, with `springboard::pngenc::adler32` for the trailer).

- `Algorithm`: the reference cases `Brotli`, `Lz4`, `LzBitmap`, `Lzfse`, `Lzma`, `Zlib`, plus `Deflate`
  (raw RFC 1951, a BifrOSt addition). Only `Zlib` and `Deflate` have a codec; the others return
  `Error::Unsupported`.
- Single-step buffers: `encode_buffer`, `decode_buffer`, `decode_buffer_limited` (output limit, default
  `inflate::MAX_OUT`). Zlib decoding checks the header (method 8, header checksum divisible by 31, no
  preset dictionary). The Adler-32 trailer is not verified on decode.
- Multiple-step streams: `Stream::init(FilterOperation, Algorithm)` and `Stream::process(src, finalize)`.
  Input is buffered; the codec runs on `finalize`, and the output is returned by that call. The stream
  does not produce output in pieces.
- Filters: `InputFilter::new(op, algorithm, buffer_capacity, reading_from)` with `read_data(length)`, and
  `OutputFilter::new(op, algorithm, buffer_capacity, writing_to)` with `write(data)` and `finalize()`. The
  source is drained on the first read; the sink receives output in chunks of the buffer capacity.
- Errors: `Error` (buffer and stream: `Unsupported`, `InvalidData`, `Truncated`, `TooBig`, `Finished`) and
  `FilterError` (`InvalidData`, `InvalidState`, mirroring the reference enum). Both implement `Display`.
- Tests: 8 `#[cfg(test)]` behaviour tests in the module, including the hand-computed empty zlib stream
  (`78 9c 03 00 00 00 00 01`), the Adler-32 trailer of `"abc"` (`02 4d 01 27`), the header rejection
  cases, the output limit, the stream finalize rule, and the filter chunking.

Not done: a true incremental encoder or decoder (the codecs work on whole buffers), Brotli, LZ4, LZBITMAP,
LZFSE and LZMA codecs, zlib trailer verification on decode, and compression levels. The zip archive
create and extract features stay in the Files app (`springboard/src/files/zip.rs`); they are not part of
this SDK module.

Build not run (wave rule). Module registration: one `pub mod compression;` line in `springboard/sdk/src/lib.rs`.

## Official API names mirrored

Official pages (DocC via `tools/parity/docc.py`, rendered via `tools/parity/rea-page.sh`):

| Official symbol | Official URL | BifrOSt name |
|---|---|---|
| Compression (framework landing) | https://developer.apple.com/documentation/compression | module `compression` |
| Algorithm | https://developer.apple.com/documentation/compression/algorithm | `Algorithm` |
| FilterError | https://developer.apple.com/documentation/compression/filtererror | `FilterError` |
| FilterOperation | https://developer.apple.com/documentation/compression/filteroperation | `FilterOperation` |
| InputFilter | https://developer.apple.com/documentation/compression/inputfilter | `InputFilter` |
| OutputFilter | https://developer.apple.com/documentation/compression/outputfilter | `OutputFilter` |
| compression_encode_buffer / compression_decode_buffer | https://developer.apple.com/documentation/compression (Single-step compression) | `encode_buffer`, `decode_buffer` |
| compression_stream, _init, _process | https://developer.apple.com/documentation/compression (Multiple-step compression) | `Stream::init`, `Stream::process` |

Mapping notes: the reference `InputFilter.readData(ofLength:)` is `read_data(length)`; `OutputFilter.write(_:)`
and `finalize()` are `write` and `finalize`. The reference FilterError cases `invalidData` and `invalidState`
are `FilterError::InvalidData` and `FilterError::InvalidState`. The reference Algorithm cases are the six
listed above, in the same spelling. Scratch-buffer sizing is not mirrored (Vec output needs none).

## REA evidence

- Compression landing: `ev_bab873a0c569fafc8c13f92a4d6c112b93626a239236f12b5c7a22686af8b39f`
- InputFilter: `ev_524fb8d6e069ddadbdd17dae70ff064e6e2e13dc2608b27abee5793a10285f31`
- OutputFilter: `ev_499ed5d92bd82da68bc9c5dedb7e1bde4035137ddcb6358ffaba668db3f417f3`
- FilterError: `ev_fb6c64ac5ba383d8528d92a0a8a3fddd1836a232a499487d0bf2ac8dc15ab6b0`
- FilterOperation: `ev_c2c53cacb39fa6158fda711b0ab868e7d0cd71b750e9aa05e5bb437cdb0cce94`
- Algorithm: `ev_7b36d63451ae17d391ddf93e235f8a035c742b4e58509699c24ebd4a71daaaaa`

## Matrix rows closed (for the lead to apply centrally)

Framework slug: `compression`. Each row moves to status `partial` (BifrOSt path given).

| framework slug | feature (exactly as in data/compression.json) | new status | BifrOSt path |
|---|---|---|---|
| compression | SDK compression API for apps | partial | springboard/sdk/src/compression.rs |
| compression | Multiple-step stream compression | partial | springboard/sdk/src/compression.rs (Stream) |
| compression | Input and output filters | partial | springboard/sdk/src/compression.rs (InputFilter, OutputFilter, FilterError) |
| compression | Error reporting | partial | springboard/sdk/src/compression.rs (Error, FilterError) |

`Error reporting` was `os`; it moves to `partial` because the new module adds encode-side error reporting
that the system layer lacked.
