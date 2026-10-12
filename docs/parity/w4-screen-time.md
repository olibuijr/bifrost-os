# Wave 4: Screen Time web usage and web history (pure models)

Framework: Screen Time, https://developer.apple.com/documentation/screentime (macOS 11.0+, Mac Catalyst 14.0+).
Official landing page: "Share and manage web-usage data, and observe changes made by a parent or guardian."

## What was built

Two pure Rust models in `springboard/ui/src` (`bifrost_ui`), std only, with no I/O, no unsafe and no network.
They are not yet wired into the browser app, the shell or the SDK, so apps cannot reach them yet.

- `springboard/ui/src/web_history.rs` (`WebHistory`, `Visit`): records a visit (bundle identifier, URL, time in
  seconds), fetches all visits, the visits of one app, or a half-open time range (`from..to`), oldest first, and
  deletes all visits, one app's visits, or a time range. Each delete returns the number removed.
- `springboard/ui/src/webpage_usage.rs` (`WebpageUsage`): the state of one page as a usage reporter holds it: URL,
  bundle identifier, profile identifier, playing video, picture-in-picture, and suppress recording. `is_blocked`
  asks the existing `url_filter::UrlFilter` (deny rules) whether the URL is denied. `report` writes one visit to a
  `WebHistory` at a given time, only when recording is not suppressed and the URL and bundle identifier are set.

Both modules are registered with one `pub mod` line each in `springboard/ui/src/lib.rs`.

## Official API names mirrored

- `STWebpageController`: `url`, `urlIsBlocked` (`is_blocked`), `suppressUsageRecording`
  (`suppress_recording`/`set_suppress_recording`), `setBundleIdentifier(_:)`, `profileIdentifier`.
  `urlIsPlayingVideo` and `urlIsPictureInPicture` map to `playing_video` and `picture_in_picture`.
- `STWebHistory`: `init(bundleIdentifier:)` maps to the `bundle` argument of `record`, `fetch_for` and
  `delete_for`; `init(profileIdentifier:)` is not mirrored (no profile store). `deleteAllHistory()` maps to
  `delete_all`, `deleteHistory(during:)` to `delete_during`, `deleteHistory(for:)` to `delete_for`,
  `fetchAllHistory` to `fetch_all`, `fetchHistory(during:)` to `fetch_during`. The official calls take completion
  handlers; the models return values directly because they are synchronous and pure.

## REA evidence

- `ev_016d90dafee0ac0fa640f30b00c51635c5c5b6f541d4800f43ea48ff19db7b3d`: rendered page
  https://developer.apple.com/documentation/screentime (headings STWebpage Controller, STScreen Time Configuration
  Observer, STScreen Time Configuration, STWeb History).
- Official DocC read with `python3 -I tools/parity/docc.py screentime`.

## Rows closed (new status partial)

| Framework slug | Feature (as written in data/screen-time.json) | Status | BifrOSt path |
|---|---|---|---|
| screen-time | Web-usage reporting (STWebpageController) | partial | springboard/ui/src/webpage_usage.rs (with springboard/ui/src/url_filter.rs and springboard/ui/src/web_history.rs) |
| screen-time | Web-history deletion (STWebHistory) | partial | springboard/ui/src/web_history.rs |

Partial, not parity: the models are host-tested only (not yet built or run here), are not connected to the browser
app or the Screen Time shell, do not persist, and have no profile store, no playing-video or picture-in-picture
effect on recording, and no completion-handler API for apps. The configuration observer and the shell's own limits
are unchanged; they remain `os` rows.

## Tests

Each module has a `#[cfg(test)] mod tests` with hand-worked expectations: 7 in `web_history.rs` (ordering, empty
input rejection, per-app fetch, half-open ranges, counted deletes) and 6 in `webpage_usage.rs` (defaults, flag
round trip, block filter verdicts, one visit written only with URL and bundle, suppression, clearing a URL).
Not run in this round, per the wave rule (no builds).
