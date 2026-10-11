# Theme 12: media item queries and filters

## What was built

`springboard/ui/src/media_query.rs` (`bifrost_ui::media_query`), a pure model of a media library query:

- `LibraryItem`: one library record (title, artist, album, album artist, composer, genre, podcast title,
  playlist names, play count, rating 0 to 5, duration, track number). The caller supplies the records.
- `Property` and `Value`: the properties a predicate tests. Text properties compare text; numeric properties
  (play count, rating, playback duration) compare numbers.
- `Predicate` and `Comparison` (`EqualTo`, `Contains`): one filter on one property.
- `Query`: a set of predicates combined with AND, at most one `Grouping` (title, album, artist, album artist,
  composer, genre, podcast title), presets (`songs`, `albums`, `artists`, `composers`, `genres`), add and remove
  of filters, `items` (matching indices in grouping order, then track number, then title) and `sections`
  (a title and a range per run of equal keys).

The model is not wired into the Music app yet. The Music app's browse filters in
`springboard/apps/music-desktop/src/music/browse.rs` stay private to that app.

## Official API names mirrored

Official documentation: https://developer.apple.com/documentation/mediaplayer/mpmediaquery

- `MPMediaQuery` (`songs()`, `albums()`, `artists()`, `composers()`, `genres()`, `filterPredicates`,
  `addFilterPredicate(_:)`, `removeFilterPredicate(_:)`, `groupingType`, `items`, `itemSections`):
  `Query`, its presets, `add_filter`, `remove_filter`, `grouping`, `items`, `sections`.
- `MPMediaPropertyPredicate` (`init(value:forProperty:)`, `init(value:forProperty:comparisonType:)`):
  `Predicate::equal_to`, `Predicate::new`.
- `MPMediaPredicateComparison` (`equalTo`, `contains`): `Comparison::EqualTo`, `Comparison::Contains`.
  `contains` follows the usual reading (the item value contains the predicate text). The official
  wording reads the other way, so the choice is a known difference, unverified on a device.
- `MPMediaGrouping` (`title`, `album`, `artist`, `albumArtist`, `composer`, `genre`, `podcastTitle`):
  `Grouping`. `playlist` grouping is not implemented.
- `MPMediaQuerySection` (`title`, `range`): `Section`.
- `MPMediaItem` property keys (`title`, `album`, `artist`, `albumArtist`, `composer`, `genre`, `playCount`,
  `rating`, `playbackDuration`, `podcastTitle`, `playlist` membership): the `LibraryItem` fields.

## REA evidence

| Page | Evidence id |
|---|---|
| MPMediaQuery | ev_7667144f974d455592d745b67f8ede3e0dae353ca82fbc00bb0e5fd21cd957fe |
| MPMediaPropertyPredicate | ev_d7f35e3d657ffae40235401759b2df391b59a41eddda0eb169020e6840e2bac3 |
| MPMediaGrouping | ev_660ec29a56e7bd7bdbcc5bea6392433eb2b4327bc24aef9281f0d9d6b18cde15 |
| MPMediaPredicateComparison | ev_748b68a832b259d784d6a211477da5dfdb3578b7c25a1c7bc1936b8c1846cf66 |
| MPMediaQuerySection | ev_18ae613598e260d35c8e4e3e226ba92ffa08fd90788f23202f6168d8d6f90c15 |

## Matrix rows closed

| Framework slug | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| media-player | Media item queries and filters | partial | springboard/ui/src/media_query.rs |

## Gaps that remain

- No SDK API connects apps to the device library. The caller passes records, so the model does not query
  the Music library.
- Collections (`collections`, `MPMediaItemCollection`) are not modelled. A group is a section over one sorted list.
- No playlist grouping and no playlist sections; playlist membership is a filter only.
- No media kind property, so songs and podcasts cannot be told apart and the `songs()` preset does not
  filter to music items.
- No live updates while a query runs.
- The `Contains` direction is an assumption noted above.
- The Music app does not use the model yet.

## Verification

No build was run (theme rule). `python3 tools/check-names.py` reports no hits for the new module.
The unit tests in `media_query.rs` are the behaviour checks and have not been run.
