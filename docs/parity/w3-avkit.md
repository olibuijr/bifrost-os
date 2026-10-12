# Wave 3: AVKit gap rows (Picture in Picture, content proposals)

Two AVKit gap rows are built as pure Rust models in `springboard/ui/src` (bifrost-ui). Both are std only, have no
hardware or network access, and are not yet wired into the shell or into an SDK app API.

## What was built

### `springboard/ui/src/pip_window.rs` (Picture in Picture)

- `Controller`: life cycle of one floating video window (`Inline`, `Starting`, `Active`, `Stopping`) with delegate
  events (`WillStart`, `DidStart`, `WillStop`, `DidStop`). The player reports whether the window is possible; turning
  that off while the window is up takes it down at once. The system can suspend the window; a suspended controller
  does not start. `enter_background(playing)` starts the window automatically when `start_automatically` is set.
- `window_size`: width is a quarter of the usable screen, clamped to 240 to 480 points, and the height is at most half
  the screen height at the video aspect ratio.
- `place` and `nearest_corner`: the window rectangle in a corner of the usable screen area with a 16 point margin, and
  the snap corner for a dropped window.
- The Settings toggle `pip_auto` (`springboard/src/shell/options.rs:72`) is the input that a shell would pass as
  `start_automatically`. It is not wired yet.

### `springboard/ui/src/content_proposal.rs` (content proposals)

- `Proposal`: the next item, with the content time at which it is offered, title, optional URL and the automatic
  acceptance interval.
- `Session`: `Waiting`, `Offered`, `Accepted`, `Rejected`. The offer appears when the content time reaches the
  transition time, or at the end of playback if that came first. It is accepted by the viewer, or automatically
  once the interval has passed since playback ended. `remaining` gives the countdown. A decided offer does not change.

## Official API names mirrored

Official pages read through `python3 -I tools/parity/docc.py` and rendered through `tools/parity/rea-page.sh`:

- `AVPictureInPictureController` (macOS 10.15, https://developer.apple.com/documentation/avkit/avpictureinpicturecontroller):
  `startPictureInPicture()`, `stopPictureInPicture()`, `isPictureInPicturePossible`, `isPictureInPictureActive`,
  `isPictureInPictureSuspended`, `canStopPictureInPicture`, `canStartPictureInPictureAutomaticallyFromInline`,
  `requiresLinearPlayback`, and the delegate events. Mirrored as `Controller` methods and `Event`.
- `AVContentProposal` (https://developer.apple.com/documentation/avkit/avcontentproposal): `contentTimeForTransition`,
  `title`, `url`, `automaticAcceptanceInterval` ("the interval between the time playback ends and automatic
  acceptance of this content proposal"). Mirrored as `Proposal` fields and `Session::tick`.
- Not mirrored: the picture-in-picture template button images, `previewImage`, `metadata`, the sample buffer playback
  delegate, and the `AVContentProposalAction` documentation, which did not load from the DocC endpoint in this run.

## REA evidence

- AVKit landing page: `ev_0459340f284ee8c3fe8b33008dc271a4000f942a307e41eba6d888a4151935eb`
- AVPictureInPictureController: `ev_46499800a6d11c17f5bad18f73e6825fefc8f8dcff74216068f6c1a2458506df`
- AVContentProposal: `ev_9180a28c28e5c266e2fe7d8936f91463f41986206b759a4e71e3fa59492f0e80`

## Matrix rows closed (new status: partial)

| Framework | Feature (as in `docs/parity-matrix/data/avkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| AVKit | Picture in Picture | partial | `springboard/ui/src/pip_window.rs` |
| AVKit | Content proposals and parental controls | partial | `springboard/ui/src/content_proposal.rs` |

Partial, not parity: the parental-controls part of the row has no counterpart, nothing draws the window or the offer,
and no SDK app API exposes either model yet.

## Not done

- Display management, route selection, capture hardware events and media trimming were left as gap rows. Trimming
  already has a recording-trim model (`springboard/ui/src/recording.rs`, `Trim`) that could be reused for the player
  in a later wave.
