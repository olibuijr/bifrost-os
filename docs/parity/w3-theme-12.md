# Wave 3, theme 12: audio search and playback requests

## What was built

`springboard/ui/src/media_request.rs` (`bifrost_ui::media_request`), a pure model of a playback request:

- `PlaybackRequest::parse` reads the request text into a verb (`MediaVerb::Play`, `AddToQueue`, `Search`) and
  its criteria. It reuses `MediaCriteria::classify` from `springboard/ui/src/media_search.rs`, so links and
  vague requests are handled the same way.
- `PlaybackRequest::plan` resolves the request against the app's catalogue through the existing
  `media_search::values_for` and returns a `PlaybackAction` (`Play`, `Enqueue`, `Show` or `NoMatch`).
- `PlaybackRequest::dialog` returns the short answer for the person ("Playing X by Y.", "Added 2 items to the
  queue.", "Found 2 results.", "No results for zebra.").

`springboard/ui/src/media_search.rs` is extended only by reuse; its code is unchanged. One `pub mod media_request;`
line was added to `springboard/ui/src/lib.rs`.

No speech recognition, speech output, SDK wiring or shell wiring is included. The app-facing intent dispatch in
`springboard/sdk/src/intents.rs` is not changed, so apps cannot yet receive these requests. Status is partial.

## Official API names mirrored

From the Media Intents collection (https://developer.apple.com/documentation/mediaintents): the audio search
request type (`AudioSearch`, `AudioSearch.Criteria`) and its playback request flow, from the article
"Responding to audio search and playback requests". The verbs play, add to queue and search are the request
kinds the framework describes in its overview.

REA evidence: `ev_79456b652b1f08cdfc53787474cf9580265e6c99a9cb29a17ea63edc659167e8`
(rendered page https://developer.apple.com/documentation/mediaintents).

## Matrix rows closed

| Framework slug | Feature (as in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| media-intents | Audio search and playback requests from Siri | partial | springboard/ui/src/media_request.rs (PlaybackRequest, plan) |
| media-intents | Voice and spoken dialog | partial | springboard/ui/src/media_request.rs (PlaybackRequest::dialog, text only, nothing is spoken) |
| siri-kit | Media intents (play, add to queue, search media) | partial | springboard/ui/src/media_request.rs (MediaVerb::Play, AddToQueue, Search) |

## Tests

`springboard/ui/src/media_request.rs` `mod tests` has six tests: verb parsing, play planning, enqueue dialog
(one item and a count), search dialog with the limit, no-match dialogs, and the zero-limit and boundary cases of
the prefix and suffix helpers. Expected values were worked out by hand from the fuzzy subsequence rule in
`springboard/ui/src/fuzzy.rs`. Nothing was built or run, as the task requires.

## Checks

`python3 tools/check-names.py`: no hits.
