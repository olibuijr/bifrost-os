# Media Intents parity

Source: https://developer.apple.com/documentation/mediaintents (the audio search and playback request type,
delivered to an app through App Intents). Matrix row: `docs/parity-matrix/data/media-intents.json`.

## What was built

`springboard/ui/src/media_search.rs` (module `bifrost_ui::media_search`), a pure model with no rendering and no
system calls:

- `MediaCriteria` mirrors the official `AudioSearch.Criteria` cases: `SearchQuery(String)`, `Unspecified` and
  `Url(String)`. `MediaCriteria::classify(text)` turns raw request text into a case: blank is `Unspecified`, an
  `http://` or `https://` link is `Url`, anything else is a trimmed `SearchQuery`.
- `MediaSearch` is the request (`MediaSearch::new(criteria)`).
- `MediaItem` and `MediaKind` (song, album, artist, episode) are the catalogue entries an app returns, with title,
  artist and album.
- `values_for(&search, &catalogue, limit)` is the matching step, the counterpart of the intent value query's
  `values(for:)`. A search query needs every content word (filler words such as "play", "some", "by" are dropped)
  to match some field with `bifrost_ui::fuzzy`; matches are ranked by summed score, ties keep catalogue order.
  `Unspecified` and filler-only queries return the first items as recommendations. A `Url` returns the item whose
  id equals the link.

Tests: 7 behaviour tests in `media_search::tests` (classification, multi-word matching across title and artist,
exclusion of items missing a word, ranking, recommendations and filler-only queries, link lookup, zero limit).

## Official API names mirrored

- `AudioSearch` (struct): `MediaSearch`.
- `AudioSearch.Criteria` (`searchQuery`, `unspecified`, `url`): `MediaCriteria`.
- Intent value query `values(for:)` (App Intents): `values_for`.
- Audio domain entities (song, album, artist, podcast episode): `MediaItem`, `MediaKind`.

## REA evidence

- Landing page `https://developer.apple.com/documentation/mediaintents`: `ev_14bb1cdd693352bc6a17a4079ae5846081e331592d36397ed501d807b2e696fb`
  (the framework delivers the audio search type through App Intents; "Audio Search" is the type that resolves requests).
- Article `https://developer.apple.com/documentation/mediaintents/responding-to-audio-search-and-playback-requests`:
  `ev_e25176c8a5e2cf989828569318e58c5d4caa3ff43e6c205b307964f08b76786d` (the app implements an intent value query over
  its entities and the system classifies requests into criteria).
- Matrix row evidence (previous round): `ev_a39e06deb47221d7e378a755d1f9e231b874c4f5a1b7589e72ffdd0e9beeafe4`.

## Not built and why

- No system delivery: nothing routes Siri or system audio requests to apps, and the model has no voice path. This
  needs the shell and App Intents delivery, outside pure Rust std code.
- No media fields in the criteria (genre, mood, media type): the official criteria carry system classification
  that BifrOSt does not have.
- No parameter resolution, disambiguation or confirmation (requestDisambiguation, requestConfirmation).
- No playback queue type and no SDK call for an app to publish its now-playing metadata.
- Intents run only while the app is open (docs/bifrost-ui.md:980).

## Verification

Not built or run (the build stage runs cargo). Checked: `tools/check-names.py` passes; the matrix JSON parses.
