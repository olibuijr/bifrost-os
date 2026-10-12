# Wave 3: MediaExtension gap rows

Framework: MediaExtension (https://developer.apple.com/documentation/mediaextension), macOS 15.0.
Official landing page REA evidence: `ev_9c04293c282a8f563803c899d1c54e057eee66f78223ca13f07b8217e1f73133`.
Official page for the processing parameter type REA evidence: `ev_086d797b19ac3d00422b18e325da5a8b052cf93e62e3fd7bbeb53b2e22a15a2b`.
DocC facts come from `python3 -I tools/parity/docc.py mediaextension` and `.../merawprocessingparameter`.

## What was built

1. `springboard/core/src/mediaext.rs` (std only, pure model). Mirrors the video decoder registration by codec type.
   - `FourCc`: a four-character codec type, four printable ASCII bytes, big-endian value, Display as text.
   - `DecoderEntry` (codec, implementation id, display name) and `parse_declaration`, which reads a
     `key = value` block with keys `CodecType`, `ClassImplementationID` and optional `DisplayName`.
     These text keys stand in for the property-list keys kMEVideoDecoderCodecTypeKey,
     kMEVideoDecoderClassImplementationIDKey and kMEVideoDecoderCodecInfoKey. No plist is read.
   - `DecoderRegistry`: one decoder per codec type, kept sorted, with register (duplicates refused), lookup,
     unregister and codecs. It loads no code; the caller chooses the implementation.
2. `springboard/render/src/raw_params.rs` (std only, pure model). Mirrors MERAWProcessingParameter for the
   boolean, integer and floating-point kinds.
   - `RawValue`, `RawRange`, `RawParameter` (range, initial, neutral and camera values, clamped set,
     reset, enabled flag) and `RawProcessingParameters` (ordered unique keys, set by key, reset_all).
   - Constructor values outside the range are refused; set values are clamped into the range.

Not built: list and sub-group parameters, the processor protocol, pixel buffer managers, notifications,
entitlements, and any RAW decoding. The Codec enum in `springboard/src/media/mod.rs` is unchanged, and the
registry is not yet wired into `default_decoder`.

## Matrix rows closed

| Framework | Feature (as in data/mediaextension.json) | New status | BifrOSt path |
|---|---|---|---|
| MediaExtension | Video decoder registration by codec type (kMEVideoDecoderCodecInfoKey and related property-list keys) | partial | springboard/core/src/mediaext.rs |
| MediaExtension | RAW processors (MERAWProcessor, MERAWProcessorExtension, MERAWProcessingParameter) | partial | springboard/render/src/raw_params.rs |

## Tests

Each new module has a `#[cfg(test)] mod tests` with six behaviour tests. Expected values were worked out by hand
from the rules in the code (for example `avc1` is 0x61766331, and an integer range of -100..=100 clamps 250 to 100).
No build or test run was done in this wave; the lead runs them.
