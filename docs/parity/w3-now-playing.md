# Wave 3: Now Playing parity (pure Rust models)

Framework: Now Playing, https://developer.apple.com/documentation/nowplaying (macOS 27.0+, Mac Catalyst 27.0+).

Three gap rows of the Now Playing matrix are closed as pure models in the bifrost-ui crate (`springboard/ui/src`).
They are not yet wired to the audio service, the lock-screen card or Control Center, so each row moves from
`gap` to `partial`, not `parity`.

## What was built

| Module | Mirrors (official API) | What it does |
|---|---|---|
| `springboard/ui/src/media_content.rs` | `MediaType` (audio, video), `MediaDuration` (continuous, finite(_:), live), the shared fields of `MediaContentRepresentable` and `MusicContent` (id, songTitle, artistName, albumName, type, duration, isExplicit) | Typed item metadata with trimmed, non-empty id and title; finite lengths must be positive and finite; `label()` gives `3:45`, `1:02:05`, `Live` or nothing; `progress()` gives a clamped 0 to 1 share for finite items only; `subtitle()` joins artist and album. Artwork bytes are not modelled. |
| `springboard/ui/src/playback_rate.rs` | `MediaCommand.changePlaybackRate(supported:_:)` | A sorted, de-duplicated set of supported rates. `accepts()` is an exact match. `resolve()` picks the nearest supported rate, a tie going to the slower one. `step()` moves one supported rate faster or slower and stops at the ends. |
| `springboard/ui/src/skip_interval.rs` | `MediaCommand.skipForward(preferredIntervals:_:)`, `MediaCommand.skipBackward(preferredIntervals:_:)` | A skip command with preferred intervals in seconds. The first interval is used. `target()` clamps a forward skip to a known end and a backward skip to zero. Live items have no end, so their forward skip is not clamped. |

Each module has its own `#[cfg(test)] mod tests` block. Each `lib.rs` change is one `pub mod` line in
`springboard/ui/src/lib.rs`, after `playback_arbiter`.

## Assumptions the official pages do not state

- `changePlaybackRate(supported:_:)`: the official page only says the command "changes the playback rate of the
  media". The rule that a request resolves to the nearest supported rate, with ties going to the slower rate, is
  this module's choice.
- `skipForward(preferredIntervals:_:)`: the page does not say how the list is used. This module reads it as
  preference order, with the first interval used. Clamping at the end and at zero is also this module's rule.
- `MediaDuration.finite(_:)` takes a time interval; the seconds unit and the positive-only rule are this module's
  choice, not Apple's.

## Official documentation read

- `python3 -I tools/parity/docc.py nowplaying/mediaduration`: MediaDuration cases continuous, finite(_:), live.
- `python3 -I tools/parity/docc.py nowplaying/mediatype`: MediaType cases audio, video.
- `python3 -I tools/parity/docc.py nowplaying/musiccontent`: init(id:songTitle:artistName:albumName:type:duration:artwork:) and the instance properties.
- `python3 -I tools/parity/docc.py nowplaying/mediacontentrepresentable`: the shared content properties.
- `python3 -I tools/parity/docc.py nowplaying/mediacommand`: the command factory list, including changePlaybackRate, skipForward and skipBackward.
- `python3 -I tools/parity/docc.py nowplaying/mediacommand/changeplaybackrate(supported:_:)` and `.../skipforward(preferredintervals:_:)`: one-line abstracts only.

## REA evidence

- `tools/parity/rea-page.sh https://developer.apple.com/documentation/nowplaying/mediaduration`:
  `ev_7c3aabef9b74924f07d6a0336bd391c04330d3bd5722955bd3cdf872e9e62aeb`
- The Now Playing landing page was recorded in wave 1 as `ev_b851395bbd5ed4a24947758f6d3c4a5a2f08a78adeca6e13a0413124a2145dc8`.

## Matrix rows closed

These are the rows of `docs/parity-matrix/data/now-playing.json` that go from `gap` to `partial`. The lead updates
the matrix centrally.

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| now-playing | Content types and metadata (MusicContent, PodcastContent, MovieContent, TVShowContent, BookContent, RadioContent, GenericContent, MediaType, MediaDuration) | partial | springboard/ui/src/media_content.rs |
| now-playing | Skip forward and backward by interval | partial | springboard/ui/src/skip_interval.rs |
| now-playing | Playback rate | partial | springboard/ui/src/playback_rate.rs |

## Not done in this wave

- Local sessions (MediaSession), remote sessions and the feedback commands (like and dislike) stay `gap`.
  Routing these models through the audio service needs a service-side change, and the feedback command has no
  model yet.
- The audio service track still carries no title, artist or artwork, so the new content model is not yet used by
  the lock card.
- No build, no run: the models were written without `cargo` and their tests have not been run.
