# Video decompression for apps (parity sdk-rest-01)

Mirrors the reference decompression session, hardware decode query, zero-copy output and codec set of the
video framework, for the rows `video-toolbox` "Decompression session", "Hardware decode capability query",
"Zero-copy decoded frames" and "Media extensions and codec registration".

## What is wired

- `springboard/sdk/src/video_decompression.rs` (`bifrost_app::video_decompression`):
  - `FormatDescription::new` checks the picture size (1 to 8192 per side) and the avcC or hvcC parameter record
    (version 1, fixed header present) or Annex B parameter sets.
  - `can_accept` and `DecompressionSession::create` apply the `video_decode` policy (`Preference`) before opening
    a decoder from `springboard::media::hwdec`.
  - `DecompressionSession::decode_frame` converts length-prefixed samples to Annex B, sends the parameter sets
    before the first sample and after `flush`, and delivers each `DecodedFrame` to the output handler.
  - `DecodedPixels` is NV12 copied out, RGBA, or a dma-buf `Buffer` when `set_zero_copy(true)` is honoured.
  - `hardware_support`, `hardware_decode_supported` and `support_from_probe` expose the libva probe per codec.
  - `codec_for_sample_entry` maps `avc1`, `hvc1` and the other sample entry codes to the fixed codec set.
- `springboard/sdk/src/video_decode.rs` (the existing decision model) supplies `Codec`, `HardwareSupport` and
  `Preference`.
- Tests: `springboard/sdk/tests/parity_sdk-rest-01.rs`, headless, with a `VideoDecoder` double.

## What needs the lab machine

- The libavcodec decoder and the libva probe run only where a render node and the libraries exist; no test does.
- Zero-copy pictures are not imported by the GPU path for apps.

## Not built

- VP9 and AV1 decode (refused with `SessionError::UnsupportedCodec`).
- A plug-in registration for decoders (the codec set is fixed in `springboard/src/media/mod.rs`).
- Profile and level queries from the probe.
