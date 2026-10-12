# Parity wave 3, theme 06: XMP, IPTC and 8BIM metadata

## What was built

`springboard/core/src/imgmeta.rs` (pure std, no I/O, no unsafe), registered as `pub mod imgmeta;` in `springboard/core/src/lib.rs`.
It reads and writes the three descriptive-metadata containers of a picture:

- **8BIM image resource blocks** (`parse_resources`, `write_resource`, `find_resource`): the
  `8BIM` id, pascal name and even-padded data layout, and the IPTC resource id `0x0404`.
- **IPTC-IIM datasets** (`parse_iptc`, `write_iptc`, `application_value(s)`, `set_application_text`,
  `add_application_text`): short and extended (`0x8000` flag) lengths, UTF-8 with an ISO 8859-1
  fallback, and the application record (2:05 object name, 2:25 keywords, 2:80 by-line, 2:105
  headline, 2:116 copyright, 2:120 caption).
- **Container glue** (`iptc_from_app13`, `xmp_from_app1`): the APP13 `Photoshop 3.0` header and the
  APP1 XMP header.
- **XMP properties** (`xmp_value`, `xmp_packet`): attribute and element forms, `rdf:Alt` and
  `rdf:li` items, XML entity escaping, and a writer for a fixed set of namespaces (`dc`, `xmp`,
  `photoshop`, `Iptc4xmpCore`). Unknown prefixes are dropped on write.
- **Caption** (`caption`): the XMP `dc:description`, falling back to IPTC 2:120 when the XMP value is
  absent or empty.

Limits: the XMP reader is a targeted string scanner for simple properties and language
alternatives. It is not a general RDF parser; it does not read structs, qualifiers, or
`rdf:Bag`/`rdf:Seq` beyond the first item. The writer covers only the listed namespaces. No code
writes a complete APP13 or APP1 segment; the caller places the bytes. No image decode path calls
this module yet, so SDK apps get no API for it. The Photos app EXIF reader
(`springboard/src/shell/photos/exif.rs`) is unchanged.

## Apple API mirrored

- `CGImageMetadata`: `CGImageMetadataCreateFromXMPData`, `CGImageMetadataCopyTags`,
  `CGImageMetadataCopyStringValueWithPath`, `CGImageMetadataCreateXMPData`. The module mirrors the
  XMP read and write concept, not the opaque type or its functions.
- IPTC Dictionary Keys (the application-record dataset numbers used by image metadata).
- Image caption metadata (the caption concept only; the Media Accessibility captioning calls are
  not mirrored).

Official documentation was read with `python3 -I tools/parity/docc.py imageio/cgimagemetadata`,
which lists the functions above and the XMP Metadata section.

## REA evidence

- `ev_6d2fcaa74ccbd31a0898bcf60038e3e001d505bd808bc00b6ce1184da989d1dc`
  (https://developer.apple.com/documentation/imageio/cgimagemetadata, rendered page)

## Matrix rows closed (new status partial)

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| image-i-o | XMP, IPTC and 8BIM metadata | partial | springboard/core/src/imgmeta.rs |
| media-accessibility | Image accessibility captions (metadata) | partial | springboard/core/src/imgmeta.rs (`caption`) |

## Tests

`#[cfg(test)] mod tests` in `imgmeta.rs` has seven tests, with expected values worked out by hand
from the layout rules: hand-written 8BIM bytes, odd-size padding across two blocks, IPTC short and
extended lengths and a truncated dataset, the Latin-1 fallback, the IPTC round trip with set and
add semantics, the APP13 and caption precedence, and XMP attribute, alternative and escape handling.
No cargo build or test was run in this round (wave rule).
