# Parity wave 5: AVFoundation (playback and editing)

Three pure Rust models for the two `gap` rows of AVFoundation (Playback, Editing, Common). They are in
`springboard/ui/src` (`bifrost_ui`). None is wired into an app or the shell yet, and none has been built or
run on the VM or Midget: this wave was written without builds, so the first compile is the real check.

## What was built

| Module | Mirrors (official API) | What it models |
|---|---|---|
| `springboard/ui/src/export_session.rs` | `AVAssetExportSession`, `AVAssetExportSession.Status`, `timeRange`, `fileLengthLimit`, `supportedFileTypes`, `export(to:as:)`, cancellation (modelled as `cancel`) | One export of a source: preset, supported file types, time range inside the source, a file length limit (inclusive), progress as a clamped fraction that never moves back, and the status values `unknown`, `waiting`, `exporting`, `completed`, `failed`, `cancelled`. A session starts once, and only for a supported file type. |
| `springboard/ui/src/frame_grab.rs` | `AVAssetImageGenerator`, `requestedTimeToleranceBefore`, `requestedTimeToleranceAfter`, `maximumSize`, `image(at:)` (which returns the actual time) | `pick` chooses the frame for a requested time from known frame times, inside a before/after tolerance window; the nearest candidate wins and a tie goes to the earlier frame. `fit` scales an image into a maximum size, keeps the aspect ratio, never enlarges, and rounds down. |
| `springboard/ui/src/render_synchronizer.rs` | `AVSampleBufferRenderSynchronizer`, `setRate(_:time:atHostTime:)`, `currentTime()`, `rate`, `renderers`, `addRenderer`, `removeRenderer`, boundary time observers | A timeline anchored by `set_rate` (media time, host time, rate). `current_time_us` gives the media time at a host time; changing the rate re-anchors without a jump. Renderers are attached by unique id. `boundary_crossings` returns the boundary times a host-time interval passes. |

### Choices this module makes (not stated by the official pages)

- `frame_grab`: a tie between a frame before and a frame after the request goes to the earlier frame. The default
  tolerance is zero (exact match). The official page does not give a default.
- `export_session`: a file length limit of zero means no limit. Progress reported above the range is clamped.
- `render_synchronizer`: a boundary counts when the timeline reaches it; the end point is included and the start
  point is not. The `timebase` of the official API is modelled by the anchor pair instead of an object.

### Not built

- Reading and writing samples (`AVAssetReader`, `AVAssetWriter`) and the encoder: no decode or encode happens here.
- Image output: `image(at:)` returns a frame time, not a picture. The decode path is `springboard/src/media/`.
- Export jobs running in the background, temporary directories, metadata filters, video compositions and audio
  mixes in an export. `springboard/ui/src/composition.rs`, `audio_mix.rs` and `video_transition.rs` (waves 1 and 4)
  hold the composition and mix models; an export session does not reference them yet.
- App-facing SDK API for any of the three. Shell-internal media stays as it was.

## Official references

REA evidence of the rendered official pages (`tools/parity/rea-page.sh`):

| Page | URL | REA evidence id |
|---|---|---|
| AVFoundation (framework) | https://developer.apple.com/documentation/avfoundation | `ev_07222d3f3679e2c5b6518af5cf7b73554c8090f4681453386a8915ae70940017` |
| AVAssetExportSession | https://developer.apple.com/documentation/avfoundation/avassetexportsession | `ev_0eac057fa53ab29736bebcc7fcdfad12226704421fcff34328ebd590f74a5da2` |
| AVAssetImageGenerator | https://developer.apple.com/documentation/avfoundation/avassetimagegenerator | `ev_342d875a955e985e6a56fccb60f99a02651836dbd70f09c2c6df3ee1f267b77c` |
| AVSampleBufferRenderSynchronizer | https://developer.apple.com/documentation/avfoundation/avsamplebufferrendersynchronizer | `ev_6057b8784fe9d99c58469fa59eef22bfd2c8b2816a6b9f0c8021df67034ee8f1` |

Status values, tolerance properties and synchronizer methods were read from the DocC JSON
(`python3 -I tools/parity/docc.py avfoundation/...`): `avassetexportsession/status-swift.enum` (cases `unknown`,
`waiting`, `exporting`, `completed`, `failed`, `cancelled`), `avassetimagegenerator/requestedtimetolerancebefore` and
`...after`, `avsamplebufferrendersynchronizer/setrate(_:time:atHostTime:)`, `currentTime()`.

## Matrix rows closed

Row status moves from `gap` to `partial` in each case. The lead updates `docs/parity-matrix/` centrally.

| Framework slug | Feature (as in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| avfoundation-playback-editing | Media reading and writing | partial | springboard/ui/src/export_session.rs, springboard/ui/src/frame_grab.rs |
| avfoundation-playback-editing | Sample buffer playback | partial | springboard/ui/src/render_synchronizer.rs |

Rows left as they were: Media playback, Video decoding and Audio output and routing (`os`, need an app-facing SDK
API); QuickTime movies (`gap`, no movie editing model yet).

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked expectations: 7 in `export_session`, 7 in `frame_grab`,
9 in `render_synchronizer`. Run them with `cargo test -p bifrost-ui export_session frame_grab render_synchronizer`
on the first build.
