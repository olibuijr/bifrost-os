# Apple Archive

Framework: Apple Archive, https://developer.apple.com/documentation/applearchive (matrix slug `apple-archive`).
Built on parity/team-j-apple-archive. Not yet verified on Midget.

## What is built

`springboard/sdk/src/archive.rs` (`bifrost_app::archive`), on top of the compression module:

- Folders and single files as ZIP: `compress_directory`, `compress_file` and `expand` (through the Files app
  zip code in `springboard/src/files/zip.rs`). The format is ZIP, so the archives open in the Files app and other
  ZIP tools. They are not Apple Archive containers.
- Buffers and strings: `compress`, `decompress`, `compress_text` and `decompress_text` (raw DEFLATE, caller-given
  size limit).
- Byte streams: `ByteStream` is an in-memory buffer with a cursor that implements `Read`, `Write` and `Seek`, with
  `read_at`, `write_at` and `cancel`. `compress_to` and `compress_to_threaded` write DEFLATE at the cursor, and
  `decompress_from` reads from it.
- Thread count: `compress_to_threaded` uses the dynamic-Huffman encoder over up to the given number of threads
  for inputs of 2 MiB or more (see `docs/parity/compression.md`).

## What is not built

- The Apple Archive container: the header, entry messages and the stream traits (`ArchiveStream`,
  `fileStream`). The byte formats are not implemented here.
- Encrypted archives (`ArchiveEncryptionContext`, `encryptionStream`, `decryptionStream`). Blocked: the SDK has
  no cipher (`springboard/sdk/src/crypto_key.rs` provides HMAC authentication only), and key derivation is not
  built. This row stays a gap.
- Ownership, flags and extended attributes in archive headers. Tar entries keep mode only.

## REA evidence

- Apple Archive landing page: `ev_d29483e649b181c7241be638022e1dc6ebfbd3923c5a414205cc47a344dd7a4d` (from the
  matrix row).
- ArchiveByteStream: `ev_71a78c3dfcb443ec778a6f63a304187d3d325aeba0c68705a1d78ea311bc2f57`
- ArchiveByteStreamProtocol: `ev_04b35f10e23b02386e5dc8e5866e84a4067d7917f17f9f746164d4df721557ba`

## Tests

`#[cfg(test)]` in `archive.rs`: stream write, seek and read back; zero-filled gaps; `read_at` clipping; seek before
the start; cancelled streams; empty and stored blocks; the threaded stream write from a non-zero cursor. Not built or
run, per the wave rules.
