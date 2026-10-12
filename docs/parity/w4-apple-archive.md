# Wave 4: Apple Archive (archives and compression for apps)

Framework: Apple Archive, https://developer.apple.com/documentation/applearchive (official landing page;
its topics are the archive, stream, header and error reference).

## What was built

`springboard/sdk/src/archive.rs` (module `bifrost_app::archive`, one `pub mod archive;` line in
`springboard/sdk/src/lib.rs`). It is an app-facing API over code the system layer already had:

- `compress` / `decompress(data, max_len)`: raw DEFLATE of byte buffers, using the Files app's
  LZ77 and fixed-Huffman encoder (`springboard::files::zip::deflate`) and the Books engine's inflate
  (`springboard::shell::books::engine::inflate::inflate_limited`). Decoding is bounded by `max_len`.
- `compress_text` / `decompress_text`: the same for UTF-8 strings; invalid UTF-8 is refused.
- `compress_file` / `compress_directory(src, dest_dir, progress)`: ZIP archives written by the Files app
  creator (`springboard::files::zip::create`), with the same progress callback and cancel return.
  A missing path, or the wrong kind (file for the folder call, folder for the file call), is `InvalidInput`.
- `expand(archive, dest, progress)`: the Files app extractor (`zip::extract`), returning `Extracted`
  with the skipped unsafe member names.

Tests: 7 in `#[cfg(test)] mod tests` in `archive.rs`. Expected values were worked by hand from RFC 1951
(an empty fixed block is `03 00`; a stored block is a header byte, LEN, NLEN = !LEN, then the bytes).
Not run: this wave is not allowed to build.

Not built: streams (`ArchiveStream`, `ArchiveByteStream`), encryption, the Apple Archive container
format (headers, ownership, xattrs), and multithreaded compression. Those stay as gap rows.

Important limit: the file archives are ZIP, not the reference archive format. ZIP stores the file
name, times and CRC, but not ownership, flags or extended attributes.

## Official API names mirrored

Official landing page topics, mirrored by the functions above:

- Compressing single files; Decompressing single files: `compress_file`, `expand`
- Compressing file system directories; Decompressing and extracting an archived directory:
  `compress_directory`, `expand`
- Compressing and saving a string to the file system; Decompressing and parsing an archived string:
  `compress_text`, `decompress_text` (byte form: `compress`, `decompress`)

The official topic pages for the articles above did not resolve at the guessed URLs (see REA below),
so the names come from the landing page topic list only.

## REA evidence

- `ev_a06c08167d81dc0f480c9f4b2353bc0cbda72f56de6fde2d0a6131296d7ce371`: "Apple Archive | Apple Developer
  Documentation", the rendered landing page. Its topic list matches the table above.
- `ev_61c839d6170a657a50ce429322c305914e3af761c2c3a7b31a19b8beeb29b9bf`: guessed article URL
  `.../applearchive/compressing-file-system-directories` rendered "page you're looking for can't be found".
- `ev_f0228a42ee2bb249b7484d8bcace78321c0ed51f97f45ec3f0e9b9a8033eb799`: guessed article URL
  `.../applearchive/compressing-and-saving-a-string-to-the-file-system` rendered the same not-found page.

## Matrix rows closed

Feature text is exactly as written in `docs/parity-matrix/data/apple-archive.json`.

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| apple-archive | Compressing file system directories | partial | springboard/sdk/src/archive.rs (compress_directory, expand) |
| apple-archive | Compressing and decompressing single files | partial | springboard/sdk/src/archive.rs (compress_file, expand) |
| apple-archive | Compressing and parsing strings | partial | springboard/sdk/src/archive.rs (compress_text, decompress_text, compress, decompress) |

The framework slug is `apple-archive` (`docs/parity-matrix/data/apple-archive.json`). The matrix rows
for "Compressing file system directories" and "Compressing and decompressing single files" were status
`os`; the row for "Compressing and parsing strings" was `gap`. The lead applies the status change centrally.
