# Parity sdk-rest-12: video decoder choice (Media Toolbox row)

Mirrors the decision that the shell's media module makes for each video stream: which decode path it gets.

## What is wired

- `springboard/ui/src/video_decoder_plan.rs`: `VideoCodec::from_sample_entry` (`avc1`, `avc3`, `hvc1`, `hev1`),
  `choose` (hardware when the software switch is off, the decoder library matches and the GPU supports the codec,
  software otherwise) and `DecodePath::fallback` (one step from hardware to software).
- The rule matches `springboard/src/media/hwdec/ffmpeg.rs` (`Decoder::open`), which uses the same three inputs and
  reopens in software when the GPU rejects a stream.
- Tests: `springboard/ui/tests/parity_sdk-rest-12.rs`, headless.

## What needs the lab machine

- The support probe (`springboard/src/media/hwdec/vaapi.rs`) reads the real GPU, so the codec support it reports is only
  checked on the lab machine.
- Apps still get no decoder API. The decoder remains internal to the shell, and this model is read-only for apps.
