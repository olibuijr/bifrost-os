# audio-01: bifrost-audio-types

Implemented: `StreamFormat` (`float32`, `integer`, `frames_for`, `bytes_for`), `TimeStamp` extrapolation with the rate
scalar (0 is treated as 1.0) plus `BitOr` on `TimeStampFlags`, `BufferList` (`silent`, `frames`, `interleave_into`,
`deinterleave_from`; short slices never panic), address tests, and the channel layout moved unchanged from
`springboard/ui/src/channel_layout.rs` into `springboard/audio/types/src/layout.rs`. The ui module re-exports it and
`bifrost-ui` now depends on `bifrost-audio-types`. `lib.rs` also re-exports `Label`, `LayoutError` and `Tag`.

Additive contract changes: `impl BitOr for TimeStampFlags`; the extra re-exports.

Note: `bytes_for` counts all channels for both layouts (for non-interleaved formats `bytes_per_frame` is per channel).

Left: nothing on the host. No Midget steps (pure types). The lead runs
`cargo test -p bifrost-audio-types -p bifrost-ui`.
