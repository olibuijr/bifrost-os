# Wave 5: Apple Archive (archive streams and DEFLATE)

Framework: Apple Archive, https://developer.apple.com/documentation/applearchive (matrix slug `apple-archive`).

## What was built

`springboard/sdk/src/archive.rs` (extends the wave 4 module; no new module file):

- `ByteStream`: an in-memory byte buffer with a cursor. It implements `std::io::Read`, `Write` and `Seek`,
  with `read_at` and `write_at` that leave the cursor alone. Writing past the end fills the gap with zeros.
  A seek before the start is `InvalidInput`. `cancel()` makes later reads, writes and flushes fail with `Interrupted`.
- `compress_to(stream, data)`: writes the raw DEFLATE of `data` at the cursor and returns the byte count.
- `decompress_from(stream, max_len)`: inflates everything from the cursor to the end, bounded by `max_len`.

The DEFLATE encoder and decoder are the wave 4 `compress` and `decompress` (shell inflate, bounded output),
so the `DEFLATE decoding` row is now reachable from apps through the SDK.

## Official API names mirrored

- `ArchiveByteStream` (class, "An archive stream that reads from and writes to buffers"): `ByteStream`.
- `ArchiveByteStreamProtocol` (`read(into:)`, `read(into:atOffset:)`, `write(from:)`, `write(from:atOffset:)`,
  `seek(toOffset:relativeTo:)`, `cancel()`, `close()`): `ByteStream::read`, `read_at`, `write`, `write_at`,
  `seek`, `cancel`, and `into_bytes` (close).
- `compressionStream(using:writingTo:...)` and `decompressionStream(readingFrom:...)` (the byte-buffer
  compression pair): `compress_to` and `decompress_from`. Compression is raw DEFLATE, not the Apple Archive
  compression algorithms, and there is no block size, flags or thread count.

## Not built

- Encrypted archives (`ArchiveEncryptionContext`, `encryptionStream`, `decryptionStream`): needs a cipher and
  key handling, which pure std Rust does not provide. Left as a gap.
- The Apple Archive container format (header, entry messages, the `ArchiveStream` traits, `fileStream`).
- Multithreaded compression (`threadCount`); the encoder is single-threaded.
- Ownership, flags and extended attributes in archive headers.

## REA evidence

- ArchiveByteStream: `ev_71a78c3dfcb443ec778a6f63a304187d3d325aeba0c68705a1d78ea311bc2f57`
- ArchiveByteStreamProtocol: `ev_04b35f10e23b02386e5dc8e5866e84a4067d7917f17f9f746164d4df721557ba`
- Apple Archive landing page (from wave 4): `ev_d29483e649b181c7241be638022e1dc6ebfbd3923c5a414205cc47a344dd7a4d`

## Matrix rows closed (for the lead to apply centrally)

| Framework slug | Feature (as in data/apple-archive.json) | Old status | New status | BifrOSt path |
|---|---|---|---|---|
| apple-archive | Archive streams (data blobs and byte buffers) | gap | partial | springboard/sdk/src/archive.rs (ByteStream, compress_to, decompress_from) |
| apple-archive | DEFLATE decoding | os | partial | springboard/sdk/src/archive.rs (decompress, decompress_from) over springboard/src/shell/books/engine/inflate.rs |

## Tests

Host unit tests in `springboard/sdk/src/archive.rs` (`mod tests`): stream write, seek and read back; gap
zero-fill; `read_at` clipping and cursor; seek before start; cancelled stream; empty block in a stream;
round trip from the cursor and a stored block. Not built or run in this wave, per the wave rules.
