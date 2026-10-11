# Theme 03: media reading, playback options and selection (wave 3)

Pure Rust models in `springboard/ui/src/` (bifrost-ui, std only, no hardware, no services). They hold the
state and decision rules of the official media APIs. The audio service, the Now Playing bridge and the library scan are
not wired to them yet, so every row closed here is `partial`.

## What was built

| Module | What it models |
|---|---|
| `playback_rate.rs` | The supported rates of a playing item, the rate it plays at, refusal of unlisted or invalid rates, the nearest supported rate (ties go to the slower rate) and stepping up or down the list. Rates are kept in thousandths. |
| `media_feedback.rs` | Like, dislike and bookmark feedback as independent on or off states, the negative flag of a dislike, and a rating command with a finite scale whose both ends are inclusive. |
| `media_selection.rs` | A language option group with one active option (optional empty selection, default option), and `choose_track`: described video only when asked for, language by primary subtag, most matched accessibility preferences, earlier track on a tie. |
| `media_sources.rs` | Media source identity (trimmed, non-empty, source type audio, image or movie, folder flag) and load options: folder loading, include list, exclude list (exclude wins), first duplicate kept. |

Each module has a `#[cfg(test)] mod tests` with hand-worked expected values (5 to 7 tests per module). `lib.rs` gained
four `pub mod` lines after `media_search`.

## Official API names mirrored

- `MPChangePlaybackRateCommand` and `supportedPlaybackRates` (Media Player), with the Now Playing `changePlaybackRate(supported:_:)` rule.
- `MPFeedbackCommand` (`isActive`, localized titles), `MPFeedbackCommandEvent` (`isNegative`), `MPRatingCommand` (`minimumRating`, `maximumRating`).
- `MPNowPlayingInfoLanguageOptionGroup` (`languageOptions`, `defaultLanguageOption`, `allowEmptySelection`) and `MPNowPlayingInfoLanguageOption` (identifier, language tag, display name, characteristics).
- Media Accessibility characteristics for described video, dialogue transcription and music and sound description, as named in the matrix rows (`MAMediaCharacteristic...`). These names were taken from the matrix row text and were not separately fetched through DocC in this round.
- `MLMediaSource` (`mediaSourceIdentifier`, `attributes`) and `MLMediaSourceType` (audio, image, movie), with the load keys for folders, included and excluded sources of the media library.

## Evidence

Official DocC read with `python3 -I tools/parity/docc.py` for the pages above. REA rendered-page evidence (`tools/parity/rea-page.sh`):

| Page | REA evidence id |
|---|---|
| MPChangePlaybackRateCommand | ev_99019c0c852305807916b97ee1487c40272d4fc670c3da250e20fc17ba4f9623 |
| MPFeedbackCommand | ev_bd7f2275f765f4a5c7830e99443134e7b6d1b9b616acb4ee01f2a3493987c88a |
| MPRatingCommand | ev_59a66a1da6d5c00fdb8a561dc211b8232c69ddfc9ca5f3a37375deb7df7a42cb |
| MPNowPlayingInfoLanguageOptionGroup | ev_273978339878fd00dd14f3cf899fdbd84d9d5660a75f09d4ea185f1dafaa214e |
| MLMediaSource | ev_a8b67c0826d3bc05ddc9031468129630e5a289b7119220f6529cbad2be1b99da |

## Matrix rows closed (status partial)

| Framework slug | Feature (as in data) | BifrOSt path |
|---|---|---|
| media-player | Playback rate and language options | springboard/ui/src/playback_rate.rs, springboard/ui/src/media_selection.rs |
| now-playing | Playback rate | springboard/ui/src/playback_rate.rs |
| media-player | Feedback and rating | springboard/ui/src/media_feedback.rs |
| now-playing | Feedback commands (like, dislike) | springboard/ui/src/media_feedback.rs |
| media-accessibility | Audible media selection (described video, audio description preference) | springboard/ui/src/media_selection.rs |
| media-accessibility | Subtitle and caption tracks in playback | springboard/ui/src/media_selection.rs |
| media-library | Media sources (MLMediaSource) | springboard/ui/src/media_sources.rs |

## Not done

- No binding to the audio service (`springboard/src/svc.rs`), the Now Playing bridge or the Control Center tile; apps cannot yet reach these models through the SDK.
- No remote command events: the feedback and rating models hold state only.
- No media library query or source scan; the library scan stays in the Music app.
- Not built or run in this round (no builds by rule); the tests are hand-checked only.
