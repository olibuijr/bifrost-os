# Wave 5: Metal gap rows (indirect commands, counter samples, HDR)

Branch `parity/w5-metal`, based on `parity/integration`. Pure Rust, std only, no new crates, no unsafe, no hardware
access. Nothing here is wired into apps, the shell or the presenter yet, and none of it has been built or run on the
VM or Midget (wave 5 is host-only, no builds in this round).

## What was built

| Module | Path | Mirrors (official API) | Behaviour |
|---|---|---|---|
| Indirect command list | `springboard/ui/src/indirect_command.rs` | `MTLIndirectCommandBuffer` (`size`, `reset(_:)`, indirect command retrieval by index) | A fixed list of `size` slots of one kind (draw or dispatch). Encode by index, reset a range to empty, execute a range in order. Execution is checked as a whole: an empty slot in the range refuses the call and runs nothing. A dispatch cannot be encoded into a draw list. |
| Counter sample buffer | `springboard/ui/src/counter_sample.rs` | `MTLCounterSampleBuffer` (`sampleCount`, `label`, `resolveCounterRange(_:)`) | A labelled buffer of timestamped samples. Record by slot, resolve a range to values, or to the intervals between consecutive values. A read is refused as a whole when a slot is unwritten or when values go backwards. |
| HDR transfer curves and EDR headroom | `springboard/render/src/hdr_transfer.rs` | "Displaying HDR content in a Metal layer" (extended dynamic range: values above the SDR reference white, adapted to the display) | Perceptual quantizer (SMPTE ST 2084) encode and decode between nits and code values; hybrid log-gamma (ARIB STD-B67) encode and decode; `EdrHeadroom` holds the SDR reference white and the peak, and clamps a luminance to `0..=headroom` as a multiple of the reference white. |

`springboard/ui/src/lib.rs` gains `pub mod indirect_command;` and `pub mod counter_sample;`.
`springboard/render/src/lib.rs` gains `pub mod hdr_transfer;`.

Each module has a `#[cfg(test)] mod tests` with 8 to 9 behaviour tests. The expected values were worked out by hand
from the formulas: for example `pq_decode(0.5)` is about 92.3 nits (the test accepts 92.0 to 92.6), `hlg_encode(1/12)`
is 0.5, and `headroom(100 nits, 1000 nits)` is 10.

Not built here: ray tracing, acceleration structures, ML passes, the GPU capture manager (`MTLCaptureManager`),
GPU counter sets and counter resolve from a real device, and any wiring of the HDR curves into the presenter, the
video path or the buffer metadata in `springboard/render/src/buffer_attach.rs` (its `Transfer::Pq` and `Transfer::Hlg`
are tags only, with no curve applied).

## Official API names mirrored

- MTLIndirectCommandBuffer: https://developer.apple.com/documentation/metal/mtlindirectcommandbuffer
  (size; reset(_:); indirectRenderCommandAt(_:) and indirectComputeCommandAt(_:) to retrieve commands)
- MTLCounterSampleBuffer: https://developer.apple.com/documentation/metal/mtlcountersamplebuffer
  (label; sampleCount; resolveCounterRange(_:))
- Displaying HDR content in a Metal layer: https://developer.apple.com/documentation/metal/displaying-hdr-content-in-a-metal-layer
  (extended dynamic range; values above the standard range that adapt to the display's capability)

Apple's MTLCaptureManager page (https://developer.apple.com/documentation/metal/mtlcapturemanager) was read for this
wave, but nothing from it is built.

## REA evidence

| Page | Evidence id |
|---|---|
| https://developer.apple.com/documentation/metal/mtlindirectcommandbuffer | `ev_72975d736ee4eea8c2ae322f483c860cb6ebab3d5218cb0be6ddf4cbc75f89c4` |
| https://developer.apple.com/documentation/metal/mtlcountersamplebuffer | `ev_db1088b07f91c8e6b1ebbf97371e9ebb1943e43d9e4a1b2e4f5585cfef68d342` |
| https://developer.apple.com/documentation/metal/displaying-hdr-content-in-a-metal-layer | `ev_04ddb281dfae4ec3b84756876e670f077f456accb682bd018f8e5c8b28c62888` |

## Matrix rows closed

Status set to `partial` for each. Rows are as written in `docs/parity-matrix/data/metal.json`; the lead updates the
matrix centrally.

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| metal | Indirect command encoding and ray tracing | partial | springboard/ui/src/indirect_command.rs (indirect command list; no ray tracing, no acceleration structures) |
| metal | GPU profiling, counters and frame capture | partial | springboard/ui/src/counter_sample.rs (counter sample buffer model; no capture manager, no device counters) |
| metal | HDR presentation | partial | springboard/render/src/hdr_transfer.rs (PQ and HLG curves and EDR headroom; presenter still 8-bit SDR) |

## Notes for the lead

- The matrix row for ray tracing is still open; this wave closes only the indirect-command part of its row.
- The HDR row is still open for the presenter: the curves exist, but nothing in the presenter or the video path calls them.
