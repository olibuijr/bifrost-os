# Wave 4: Now Playing (local sessions, commands, artwork)

Parity work for the official Now Playing framework (`docs/parity-matrix/data/now-playing.json`). Two pure models
in `bifrost-ui` and one app-facing re-export in the SDK close four matrix rows. Nothing is wired into the shell or the
audio service yet, and nothing was built or run in this wave.

## What was built

- `springboard/ui/src/media_session.rs`: a local session model. `MediaSession` holds an identifier (a trimmed,
  non-empty `String`, as the official `id` is a `String`), the `canBecomeApplicationPrimary`,
  `isApplicationPrimary` and `isSystemPrimary` flags, the content (`MediaContent` from `media_content.rs`), and the
  playback state and position. The position is capped at the duration of finite content and kept for live content.
  Each playback command (`RemoteCommand`: play, pause, stop, toggle play/pause, next, previous, skip forward, skip
  backward, seek) is declared `Unsupported`, `Disabled` or `Enabled` (`CommandState`). `dispatch` answers whether a
  command from the system may run. The asynchronous `requestToBecomeApplicationPrimary()` is a synchronous
  `Result` here; the system grant for the system primary is stored with `set_system_primary`.
- `springboard/ui/src/media_artwork.rs`: an artwork model. `Artwork` has a trimmed identifier and the image sizes
  the item can supply (sides from 1 to `MAX_SIDE` = 16384). `choose` returns the smallest size that covers a target,
  or the largest size when none covers it. `aspect_fit` sizes a box in whole pixels, rounded down. `request_pixels`
  turns points and a percent scale into the pixel size a provider is asked for, rounded up. No image bytes are decoded.
- `springboard/sdk/src/now_playing.rs`: re-exports the two models for apps.

Both `lib.rs` files got one `pub mod` line each.

## Official API names mirrored

- Now Playing framework: https://developer.apple.com/documentation/nowplaying
- `MediaSession` (init, `id`, `canBecomeApplicationPrimary`, `isApplicationPrimary`, `isSystemPrimary`,
  `requestToBecomeApplicationPrimary()`, `requestToBecomeSystemPrimary()`):
  https://developer.apple.com/documentation/nowplaying/mediasession
- `Artwork` (init with identifier and provider, `id`): https://developer.apple.com/documentation/nowplaying/artwork
- Playback commands and content types are named on the framework page (article "Playback commands").

## REA evidence

- Framework page: `ev_a1d15f01b426c38d99e895c98309c2747d407b89626564fa052c47ca8519d615`
- `MediaSession` page: `ev_9b9f5c2cd3bffc6bc554faf8331787f36a2b1b8b6f75a8e71f8fc512f3457c2f`
- `Artwork` page: `ev_aae072e7c198f3a6e6f64f35813a50c6526f07353b79b17063166e951d646143`

## Matrix rows closed (new status `partial`)

| Framework slug | Feature (as in data/now-playing.json) | Status | BifrOSt path |
|---|---|---|---|
| now-playing | Local sessions: MediaSession and MediaSessionRepresentable | partial | springboard/ui/src/media_session.rs, springboard/sdk/src/now_playing.rs |
| now-playing | Artwork (Artwork, ArtworkRepresentation, AnimatedArtwork) | partial | springboard/ui/src/media_artwork.rs, springboard/sdk/src/now_playing.rs |
| now-playing | Play, pause, stop and toggle commands | partial | springboard/ui/src/media_session.rs, springboard/sdk/src/now_playing.rs |
| now-playing | Next and previous track commands | partial | springboard/ui/src/media_session.rs, springboard/sdk/src/now_playing.rs |

## Still missing

- `MediaSessionRepresentable` is not a protocol here; apps set state through `MediaSession` directly.
- No publish path to the lock screen or Control Center: the audio service carries no app session, so the session
  state does not reach the system surface. "Publishing media sessions to Lock Screen and Control Center" stays `os`.
- No artwork bytes, `ArtworkRepresentation` decoding or `AnimatedArtwork`.
- Commands have no press-and-hold begin and end actions and no enablement published to the system.
- Remote sessions, `MediaDevice` and the remote extension are not built (no network services in this wave).
