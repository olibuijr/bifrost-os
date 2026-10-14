# Content type sniffing

The UI crate now exposes a bounded, pure content sniffer that recognizes common image, archive, audio/video, font,
markup, script and UTF-8 text signatures, returning the same identifiers used by `uttype`. ZIP-like containers are
identified by their shared container signature; callers may refine the answer with a filename. Tests cover binary
signatures, fixed-offset headers, text, unknown input and the 512-byte read window.

The reference `ContentKit` API was not confirmed from the bundled `magic.mgc` resource. This implementation is a small
signature model, not a database-compatible magic engine, and no file picker or preview path calls it yet. The matrix
therefore records partial coverage only.
