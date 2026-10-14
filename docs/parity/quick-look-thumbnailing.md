# Quick Look Thumbnailing parity: thumbnail error codes

Framework: Quick Look Thumbnailing (https://developer.apple.com/documentation/quicklookthumbnailing), matrix row in
`docs/parity-matrix/data/quick-look-thumbnailing.json`.

## What was built

`springboard/sdk/src/thumbnail_error.rs` (built on parity/team-j-quick-look; not yet verified on Midget):

- `DOMAIN` is `bifrost.thumbnails`. `ThumbnailErrorCode` has five codes numbered 1 to 5: `BadRequest`, `NoSuchFile`,
  `Unsupported`, `Cancelled`, `Failed`. `raw` and `from_raw` convert to and from the number; `name` gives a log name.
- `ThumbnailError { code, message }` carries the code and the shell's message. `from_state` turns a
  `thumbnails::State::Failed` into an error (other states give None). `from_request` maps a refused
  `RequestError` to `BadRequest`. `Display` and `std::error::Error` are implemented.
- `classify` sorts the shell's message into a code by its common phrases (cancelled, no such file or gone, unsupported
  or cannot be drawn), and falls back to `Failed`.

## Limits

- The shell answers a failed thumbnail with a message only, so the code is inferred from the text. The shell should
  send a code once it has one.
- The official error domain and codes are not matched. The official DocC page for the error code was not found by
  `tools/parity/docc.py` (HTTP 404 for `quicklookthumbnailing/qlthumbnailerror` and its `.code` child), so the reference
  codes were not read.
- The system-wide thumbnail gap (Files, Spotlight and the search panel drawing thumbnails) is shell work and is not
  covered here.

## Tests

`springboard/sdk/src/thumbnail_error.rs`: 5 host unit tests (code numbers round trip, message classification, failed
state versus other states, refused request, display text). Not built or run in this round; the lead builds and tests.
