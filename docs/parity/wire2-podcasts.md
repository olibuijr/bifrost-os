# Parity wire-up round 2: Podcasts

Branch `wire2/podcasts` from `main` (0.5.37). App `springboard/apps/podcasts-desktop/`, version 1.5.0. Built with no
cargo run in this round; the lead builds once.

## What is now wired

- **Feed refresh status through an async publisher sequence** (`bifrost_app::async_publisher`, the model in
  `springboard/sdk/src/async_publisher.rs`). New module `springboard/apps/podcasts-desktop/src/refresh.rs`:
  a feed that starts loading sends `Started`, and a feed that finishes sends `Finished { ok }`, on a
  `PassthroughSubject`. The app pulls the sequence with `next()` in `tick` (never blocking), counts the batch,
  and the list shows "Updating feeds, 1 of 3" in place of "Loading...". A failed feed counts as finished and
  is counted as failed. The next batch resets the counts.
  - Reached by: opening Podcasts, adding a feed or choosing a show, which starts the feed load.
  - Verify on Midget: open Podcasts (`bifrost pc midget open-app is.olibuijr.podcasts.desktop`), add a feed URL
    with two or more shows, and check the empty list shows the "Updating feeds, n of m" text while the feeds load.
    Then `bifrost pc midget screen /tmp/podcasts-refresh.png`. The failed-feed count is `Status::failed()`, which
    the list does not show yet.

## Not wired (and why)

- **Trim and export a clip** (`springboard/ui/src/media_trim.rs`, row AVKit "Media trimming and export"). The
  model holds the in-point and out-point, but Podcasts has no trim control in its player bar, and the audio
  service has no call to cut or write a file. Writing a trimmed file needs an audio decoder and encoder that
  this app does not have. Left for a later round.
- **Flashing-light dimming for video episodes** (`springboard/ui/src/flashing_lights.rs`, row Media
  Accessibility "Flashing lights detection and dimming"). Podcasts plays audio only and has no frames to
  analyse, and the model needs luminance samples from a video surface. There is no app-facing accessibility
  setting for it either (the matrix row says none exists). Not wired.

## Official and REA evidence

- Official DocC page read with `python3 -I tools/parity/docc.py combine/asyncpublisher --depth 1`: AsyncPublisher,
  its `init(_:)` (creates the sequence from an upstream publisher), `makeAsyncIterator()` and
  `AsyncPublisher.Iterator`. Page: https://developer.apple.com/documentation/combine/asyncpublisher
- No REA evidence id was recorded in this round: REA was not started for this round.

## Matrix rows

| Slug | Feature (as written in the data file) | BifrOSt path | State |
|---|---|---|---|
| combine | Async publishers | springboard/apps/podcasts-desktop/src/refresh.rs (over springboard/sdk/src/async_publisher.rs) | wired in Podcasts; a Midget check is still to do |
| avkit | Media trimming and export | springboard/ui/src/media_trim.rs | not wired |
| media-accessibility | Flashing lights detection and dimming | springboard/ui/src/flashing_lights.rs | not wired |

## Remaining gaps

- Trim control in the player bar, and an export path (needs an audio encoder and a file write from the audio service).
- Flashing-light dimming needs a video episode path and an accessibility setting for the Podcasts app.
- The failed-feed count is not shown in the list.
- The `bifrost-podcasts` entry in the Cargo lock file is not updated in this round (no cargo run); the lead's build
  updates it.

## Tests

- `springboard/apps/podcasts-desktop/src/refresh.rs` `tests`: five tests with hand-worked counts: no feeds,
  a batch of two with one finished (1 of 2), a batch ending with one failure (progress none, failed 1), the next
  batch resetting the counts (0 of 1), and events waiting for `pump`.
