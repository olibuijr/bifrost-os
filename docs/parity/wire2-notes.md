# Wire 2: Notes (apps/notes-desktop)

Scope: the multi-line text view (`ui/src/textview.rs`), data detectors (`ui/src/detect.rs`) and symbol rendering modes
(`ui/src/symbol_layers.rs`), wired into the Notes app. Owned folder: `springboard/apps/notes-desktop/`.

## What is wired

| Feature | Where | How a user reaches it |
|---|---|---|
| Flight numbers and street addresses flagged by Check Spelling and Links | `apps/notes-desktop/src/notes/parity.rs` (`check_ranges`) uses `detect::Kind::Flight` and `detect::Kind::Address` with the links, dates and phone numbers | Edit a note, More menu > Check Spelling and Links; step through the items with the find arrows |

Tests: `check_flags_flight_numbers_as_one_item` (parity.rs). Expected values worked out by hand: "FI 552" and "BA117"
are one flight item each (the word "Flight" is a dictionary word, so it is not flagged as a flight).

Version: 1.11.0 (manifest.toml, Cargo.toml, CHANGELOG.md). The lead's build will update `Cargo.lock` for `bifrost-notes`.

## Not wired, and why

- **Multi-line text view (`ui/src/textview.rs`).** The note body is the shell's `TextArea` (`springboard/src/shell/textarea.rs`),
  which already wraps, scrolls, edits and draws links. Replacing it needs a shell change outside this app's folder. Not done.
- **Data detector underlines for flights and addresses.** `TextArea::set_detect_links` runs `detect::find` with `Kind::Link` only
  (`springboard/src/shell/textarea.rs`, `detected_links`), so only links are underlined and tappable. Needs the shell file. Not done.
- **Symbol rendering modes for checklist glyphs (`ui/src/symbol_layers.rs`).** Checklist boxes are drawn by the shell's
  `TextArea` (`md::Block::Check`), not by a symbol. The layer model has no SDK call that applies it to a glyph. Not done.
- **Tapping a flight number or address.** No action is defined; the reference platform gives a flight match and an address match
  a system action that the desktop does not have. Not done.

## How to verify on Midget

1. `bifrost pc midget update --restart`, then open Notes (`bifrost pc midget open-app is.olibuijr.notes.desktop`).
2. New note, type `Flight FI 552 lands at BA117 today`, then More > Check Spelling and Links.
3. Expected: the find arrows step to `FI 552` and `BA117` as single items, and "Flight" is not one of them.
4. Screenshot with `bifrost pc midget screen`.

## Reference evidence

- `tools/parity/docc.py` lookups of `documentation/datadetection/ddmatchflightnumber` and `documentation/appkit/nstextview`
  returned HTTP 404 or a connection error from this session, so no official-doc evidence ids were recorded.
- The matrix rows already cite `developer.apple.com/documentation/datadetection/ddmatchflightnumber`; the flight kind follows the
  official flight match kind as described in `ui/src/detect.rs`.
- REA (`tools/parity/rea-page.sh`) was not run for this wire.

## Matrix rows

| Slug | Feature (as in the matrix json) | BifrOSt path |
|---|---|---|
| datadetection | Match base type and string scanning | springboard/apps/notes-desktop/src/notes/parity.rs (flight and address kinds in Check Spelling and Links) |
| appkit-text | Text Display: data detectors | springboard/apps/notes-desktop/src/notes/parity.rs (flight and address items flagged; underlines still missing) |
| appkit-text | Text Display: multi-line text view widget | not wired (springboard/src/shell/textarea.rs is outside this app) |
| symbols | Symbol weights, scales and rendering modes | not wired (checklist glyphs drawn by the shell text area) |
