# Wave 3: Media Player (remote commands)

## What was built

`springboard/ui/src/remote_command.rs` (`bifrost_ui::remote_command`), a pure model of three remote command objects
and the requests (command events) they accept:

- `PlaybackRateCommand`: the supported playback rates of a media item (sorted, deduplicated, positive only), the
  chosen rate, an enabled switch, and `nearest` for a target rate. A request must match a supported rate.
- `RatingCommand`: the minimum and maximum of a rating range, the chosen rating, and an enabled switch. A request
  must be finite and inside the range, inclusive.
- `FeedbackCommand`: the on or off state of a like, dislike or bookmark style action, its long and short titles,
  `request` and `toggle`, and an enabled switch.

`CommandError` (`Disabled`, `NotSupported`, `OutOfRange`) is the refusal every command returns. The model does not
play media and does not register handlers: the app calls `request` from its own handler and applies the value.

Registered in `springboard/ui/src/lib.rs` with one line (`pub mod remote_command;`). Eight unit tests in the module
cover sorting and deduplication, supported and unsupported rates, disabled commands, nearest-rate ties, inclusive
rating bounds, the rating start value, feedback toggling and titles.

## Official API names mirrored

Source: https://developer.apple.com/documentation/mediaplayer (DocC via `tools/parity/docc.py`):

- `MPChangePlaybackRateCommand` (`supportedPlaybackRates`), event `MPChangePlaybackRateCommandEvent` (`playbackRate`)
- `MPRatingCommand` (`minimumRating`, `maximumRating`), event `MPRatingCommandEvent` (`rating`)
- `MPFeedbackCommand` (`isActive`, `localizedTitle`, `localizedShortTitle`), event `MPFeedbackCommandEvent`
- `MPRemoteCommand` (`isEnabled`)
- `MPRemoteCommandCenter` (`changePlaybackRateCommand`, `ratingCommand`, `likeCommand`, `dislikeCommand`,
  `bookmarkCommand`), the command center's own properties, are the names these objects are reached through. The
  model has no shared command center (see gaps).

Not mirrored: `MPRemoteCommandHandlerStatus`, target and action registration, `MPFeedbackCommandEvent.isNegative`
(the model takes the on or off state directly), and the event classes as types.

## REA evidence

- `ev_b761d4241db785b88ba957bdb30f356c5d782678af0cdda668cdb19a9b25efc8`: Media Player framework landing page
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/mediaplayer`).
- `ev_73878c692a371b8e340cce84a14fda6a9aec169579dd9616dec5f86a8b32783f`: MPRemoteCommandCenter page, which lists the
  rating, like, dislike, bookmark and change-playback-rate properties this model mirrors.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/media-player.json`) | Status | BifrOSt path |
|---|---|---|---|
| Media Player | Playback rate and language options | partial | springboard/ui/src/remote_command.rs (PlaybackRateCommand; language options still missing) |
| Media Player | Feedback and rating | partial | springboard/ui/src/remote_command.rs (RatingCommand, FeedbackCommand; no system delivery of events to apps) |

## Gaps that remain

- No delivery: the model does not receive events from the shell, Control Center or hardware media keys
  (`springboard/src/bare/input.rs:826` still notes the media-key remote without mapping it).
- No shared command center and no `isEnabled` state published to the system.
- No language option commands (`MPNowPlayingInfoLanguageOption`, enable and disable language option commands).
- No skip interval, position or bookmark command types.

## Verification

No builds were run (per the wave rules). The tests were worked out by hand from the rules in the module header.
`python3 tools/check-names.py` reported no hits for the new module.
