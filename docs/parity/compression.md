# Compression

Framework: Compression, https://developer.apple.com/documentation/compression (matrix slug `compression`).
Built on parity/team-j-apple-archive. Not yet verified on Midget.

## What is built

`springboard/sdk/src/compression.rs` (`bifrost_app::compression`, pure Rust, std only):

- Single-step buffers: `encode_buffer` (default level), `encode_buffer_threaded` (level and thread count),
  `decode_buffer` and `decode_buffer_limited` (output limit, default `inflate::MAX_OUT`).
- Algorithms: `Zlib` (RFC 1950 framing) and `Deflate` (raw RFC 1951, a BifrOSt addition). The reference cases
  `Brotli`, `Lz4`, `LzBitmap`, `Lzfse` and `Lzma` exist and fail with `Error::Unsupported`.
- Streams: `Stream::init` and `Stream::process(src, finalize)`. Input is buffered; the codec runs on finalize.
- Filters: `InputFilter` (`read_data`) and `OutputFilter` (`write`, `finalize`).
- Errors: `Error` for buffers and streams, `FilterError` for filters.

The encoder is `springboard/sdk/src/compression/dynamic.rs`, a private submodule. It is greedy LZ77 over a
32 KiB window with one dynamic-Huffman block per piece. `Level::Fast`, `Level::Default` and `Level::Best` set
the hash-chain depth (8, 128 and 2048). Code lengths are limited to 15 bits (literal and distance codes) and
7 bits (code length codes) by halving symbol counts until the tree fits. Inputs of 2 MiB or more can be split
over threads: each piece becomes its own block (the last one final) and the blocks are joined bit by bit into
one DEFLATE stream. Matches do not cross piece boundaries.

The zlib header's FLEVEL bits follow the level (`0x5e`, `0x9c`, `0xda`), and the check bits keep the header a
multiple of 31.

## What is not built

- LZFSE, LZ4, LZ4_RAW, LZBITMAP, LZMA and Brotli codecs.
- Incremental codec state: streams buffer their input and output appears only on finalize.
- Lazy matching and stored-block fallback in the encoder.
- The Files app zip writer (`springboard/src/files/zip.rs`) and the PNG writer (`springboard/src/pngenc.rs`) still
  use their own encoders. Switching them is a shared-file change and is left for the lead.

## Tests

`#[cfg(test)]` in `compression.rs`: zlib header and trailer, round trips at every level, limits and unsupported
algorithms, stream and filter chunking. In `compression/dynamic.rs`: the length and distance tables, the code
length limit, canonical codes from the RFC 1951 example, run-length codes, empty and short inputs, and a threaded
round trip over 2 MiB. Not built or run, per the wave rules.

## Official names mirrored

| Official symbol | BifrOSt name |
|---|---|
| compression_encode_buffer | `encode_buffer`, `encode_buffer_threaded` |
| compression_decode_buffer | `decode_buffer`, `decode_buffer_limited` |
| compression_stream_init, _process | `Stream::init`, `Stream::process` |
| Algorithm | `Algorithm` |
| FilterError | `FilterError` |
| InputFilter, OutputFilter | `InputFilter`, `OutputFilter` |

The thread count and `Level` are BifrOSt additions with no official symbol.
