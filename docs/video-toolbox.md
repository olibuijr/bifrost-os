# Video Toolbox parity

Reference: Apple's Video Toolbox framework (`VTCompressionSession`, `VTDecompressionSession`, `VTSessionSetProperty`).
Matrix rows: `docs/parity-matrix/data/video-toolbox.json`.

## Compression session (encoding)

Module: `springboard/ui/src/video_compression.rs`, exposed to apps as `bifrost_app::controls::video_compression`
(the SDK re-exports the UI crate as `controls`). It sits on the encoder catalogue in `video_session.rs`.

Shape (follows the reference names and behaviour):

- `CompressionSession::create` opens a session on an encoder registered in the catalogue. The session takes the
  encoder's supported properties with no values set. An unknown id is `EncoderNotAvailable`, and a zero width or
  height is `Parameter`.
- `set_property` / `copy_property` use the same rules as the catalogue: unknown names, read-only names, kinds, ranges
  and name lists are checked before a value is kept.
- `prepare` records that encoding is about to start. It does no other work.
- `encode_frame` checks the frame size against the session and hands the frame to a `Backend`. The frame carries a
  `FrameOptions::force_key_frame` flag and a `source_ref` that comes back on every output for that frame.
- Output callbacks: each packet is one `Output` with its `Packet` (pts, dts, key frame, data). A dropped frame is one
  `Output` with `InfoFlags::frame_dropped`. A backend error is sent to the handler and returned to the caller.
- `complete_frames(until)` delivers the buffered packets whose presentation time is at or before `until` (all of them
  with `None`), with the `asynchronous` info flag set. Later packets stay in the backend.
- `invalidate` ends the session; every later call returns `InvalidSession`.

### Decisions

- The session encodes nothing. Codec output comes only from a `Backend`, so no new codec is added here.
- No encoder backend is registered on this platform, so `create` refuses every id until one is. Apps cannot encode
  yet, and the matrix row stays `partial` until a backend runs on Midget.
- Times are `Time { value, timescale }` with a non-zero timescale, compared by the instant they name.
- Errors are `SessionError` values, not Apple's numeric `kVT...Err` codes. The numeric codes are not mapped yet
  (matrix row "Error codes" stays `partial`).
- Not in this round: multi-pass storage and frame silos (depend on an encoder), frame processing, and HDR and
  10-bit encode settings.

### Tests

Host unit tests in `video_compression.rs` cover: encoder lookup and size checks, time comparison across timescales,
property rules, packet and source-reference delivery, dropped frames, backend errors, the force-key-frame option,
`prepare`, time-limited and full `complete_frames`, and invalidation. They have not been run in this round.
