# Wave 5, theme-03: text storage, line layout and containers

Theme: "TextKit: storage, layout manager, containers" (23 gap rows across 16 frameworks). This round builds one shared
pure model, `springboard/ui/src/textlayout.rs` (`bifrost_ui::textlayout`), and closes the two rows whose Apple
counterpart is the text storage, layout and typesetter pair. The other rows in the theme are views, Quick Look,
PDF, chart, speech and similar capabilities that need a different subsystem and stay open.

Status: code written and reviewed by hand. No build or test was run in this round (wave rule). The six tests in the
module are worked out by hand from the rules they check and have not been executed.

## What was built

`springboard/ui/src/textlayout.rs`, one `pub mod textlayout;` line in `springboard/ui/src/lib.rs`. std only.

- `TextStorage`: text plus attribute runs (byte ranges with a `u32` attribute, 0 for none).
  - `replace_characters(range, new)` shifts the runs behind an edit, clips the runs it covers, and refuses ranges that
    are out of bounds or split a character.
  - `set_attribute(range, attr)`, `attribute_at(index)`, `attribute_runs(range)` (gaps filled with 0).
  - `process_editing()` returns the pending `EditedRange` (union of edited ranges in current offsets, and the summed
    change in length) and clears it. `generation()` changes on every edit and is what layout caches compare.
- `TextContainer`: width, height (`f32::INFINITY` is unbounded), line fragment padding, maximum line count (0 is no
  limit), and exclusion rectangles. `line_fragment_rect(y, h)` returns the widest free gap in the band, inset by
  the padding.
- `suggest_line_break(text, start, width, measure)`: the typesetter's break suggestion. Breaks after the last space
  that fits; spaces hang past the width; a word wider than the line is split by characters; a newline ends the line.
- `LayoutManager`: `ensure_layout(storage, container, measure)` lays out lines top to bottom and caches the result
  under (storage identity, storage generation, container, line height). Queries: `lines()`, `visible_range()`,
  `is_truncated()`, `line_index_for_character()`, `character_index_at(x, y)` (nearest boundary),
  `rects_for_range(range)` (one rectangle per line), `runs_in_line(line)` (attribute runs with x and width),
  `used_rect()`.

Not modelled, stated in the module header: hyphenation, bidirectional runs, glyph-level positions and kerning (the
caller's `measure` decides widths), non-rectangular containers, and the empty line after a trailing newline.

## Official API names mirrored

Each name is from Apple's official documentation (DocC JSON via `tools/parity/docc.py`, rendered pages via REA).

- AppKit TextKit topic: NSTextStorage (edited(_:range:changeInLength:), processEditing(), editedRange,
  changeInLength, attributeRuns, invalidateAttributes), NSTextContainer (size, exclusionPaths, maximumNumberOfLines,
  lineFragmentPadding, lineFragmentRect(forProposedRect:at:writingDirection:remaining:)), NSLayoutManager
  (textStorage, textContainers, usedRect(for:), ensureGlyphs(forCharacterRange:), processEditing(for:edited:range:changeInLength:invalidatedRange:)),
  NSTextLayoutManager (textContainer, ensureLayout(for:), textLayoutFragment(for:), usageBoundsForTextContainer).
- Core Text: CTTypesetterSuggestLineBreak, CTTypesetter (line creation), CTFramesetter and CTFrame (lines, visible string
  range), CTLine (string index for position, offset for string index, glyph runs, string range), CTRun (string range,
  attributes, typographic bounds).

## REA evidence

- `ev_b6c7a8205c52dff87e66f33c22e575af63dbec667512e616cbdc23c77f514b45`: rendered page
  https://developer.apple.com/documentation/appkit/textkit (TextKit overview; lists the storage, container and
  layout manager classes).
- `ev_bafcd4d085085092b8e8af7becae2a1e75a9256fbe89c52037590c88dd730dc2`: rendered page
  https://developer.apple.com/documentation/coretext (Core Text overview; lists CTTypesetter, CTFramesetter, CTLine,
  CTRun, CTFrame).

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| appkit-text | TextKit: storage, layout manager, containers | partial | springboard/ui/src/textlayout.rs |
| core-text | Line and run layout objects (typesetter, framesetter, line, run, frame) | partial | springboard/ui/src/textlayout.rs |

Left open in this theme: appkit-text "Text Display: multi-line text view widget" (needs a view and input, not only a
model), and every row in the other fifteen frameworks.

## Gaps that remain

- No SDK wiring: `bifrost_app` does not expose `TextStorage` or `LayoutManager` yet, and no widget uses them.
  `RichTextView` in `springboard/sdk/src/attributed.rs` still lays out by its own wrapper.
- Measurement comes from the caller; there is no glyph advance or kerning source (`springboard/render/src/kern.rs`
  is a separate, unwired model).
- No caret or selection geometry on top of `character_index_at` and `rects_for_range`.
- No text container chaining (one container per layout manager).
- Not tested on the VM or Midget.
