# Wave 3, theme 24: accessibility text (text markers and attributed runs)

## What was built

- `springboard/ui/src/text_marker.rs` (new, pure model in bifrost-ui): `TextMarker` (an element id and a char offset)
  and `TextMarkerRange` (two markers in one element, always ordered). It covers selection and visible ranges built from
  `AxText`, intersection and union (touching ranges count as joined), inclusive containment, clamped movement, the text
  of a range (empty for a secure field), and UTF-16 conversion (`char_to_utf16`, `utf16_to_char`), which returns `None`
  inside a surrogate pair.
- `springboard/sdk/src/attributed.rs` (extended): `AxAttrRun` and `AttributedString::ax_runs()`. The export lists each
  attribute run with its UTF-16 start and length, skips empty runs, and reuses the existing `Attrs` model. Its
  tests cover one-run plain text, UTF-16 counting of an emoji, merged neighbours and empty input.
- `springboard/ui/src/lib.rs`: one `pub mod text_marker;` line.

## Official API names mirrored

- AXTextMarker, AXTextMarkerRange and AXTextAttributedString (accessibility text markers and attributed runs).
- AttributedString (Foundation), for the run model: https://developer.apple.com/documentation/foundation/attributedstring

## REA evidence

- `ev_7286b1aecad9c060572fa364907113c89f01269c4d03a43da4219c1f98aadf6b`: rendered official page for AttributedString.
- No REA evidence was captured for the text-marker symbols. The DocC index for `accessibility` has no text-marker
  topic, and the `applicationservices/accessibility` page returned a 404 page, so it is not cited.

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| application-services | Accessibility text (AXText, attributed strings, text markers) | partial | springboard/ui/src/text_marker.rs, springboard/sdk/src/attributed.rs (`ax_runs`) |

## Not closed

- Text events and caret notifications through the AT-SPI bridge are unchanged; the markers are not yet published to it.
- Braille, Core Text ruby and run delegates, PDF widgets and translation remain gaps, because they need data or services
  this pure-Rust theme does not have.
