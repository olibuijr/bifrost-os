# Theme 32: writing direction and bidirectional text

## What was built

`springboard/ui/src/bidi.rs` (bifrost-ui, pure Rust, std only): a model of the Unicode bidirectional algorithm for one
paragraph.

- `Direction` (`Natural`, `LeftToRight`, `RightToLeft`) is the writing direction of a paragraph. `Natural` takes the
  direction of the first strong character (P2 and P3).
- `Paragraph::new(text, direction)` resolves the paragraph level, the level of every character (weak rules W1 to W7,
  neutral rules N1 and N2, implicit rules I1 and I2, line rule L1 for separators and trailing whitespace), and the
  runs in visual order. `Paragraph::visual_order()` gives character indices in visual order.
- `visual_order(levels)` is rule L2 on its own. `Run { chars, level }` with `is_rtl()` is the unit a drawing adapter
  draws; odd-level runs are placed with their glyphs reversed.

Limits (recorded so they are not mistaken for parity): explicit embeddings and isolates (LRE, RLE, LRO, RLO, LRI, RLI,
FSI, PDI, PDF) are not modelled, so the whole paragraph is one isolating run sequence. Character classes come from a
range table, not the full Unicode data, so zero-width and control characters are neutral. Paragraph separators inside
the text only affect levels; the caller still splits paragraphs. The model does not yet drive the SDK rich text view
(`springboard/sdk/src/attributed.rs`) or the text buffer, so drawing right to left is a follow-up.

## Official API names mirrored

- `CTWritingDirection` (cases `natural`, `leftToRight`, `rightToLeft`), as `Direction`.
- `kCTWritingDirectionAttributeName` (the per-run writing direction attribute), as the paragraph direction plus `Run`.
- The `CTRun` writing direction of a visual run, as `Run::is_rtl()`.

Official evidence (REA, rendered Chromium on the official page):

- `CTWritingDirection`: `ev_16216922cb2649034b96faa990d6e5710977105b3c94c0a788363a5534445a87`
  (https://developer.apple.com/documentation/coretext/ctwritingdirection)
- `kCTWritingDirectionAttributeName`: `ev_7e5639dd530041d76a73268652194b473dbff8c66e9eeba79b71344e4cdb07b8`
  (https://developer.apple.com/documentation/coretext/kctwritingdirectionattributename)

## Tests

Nine behaviour tests in `bidi::tests`: Hebrew under the natural direction, digits keeping a left-to-right paragraph,
a neutral between opposite directions, numbers in a right-to-left paragraph, visual order of runs, Arabic-Indic digits,
a combining mark following its base, a mixed line ordered for display, and trailing whitespace at the paragraph level.

## Matrix rows closed

| Framework slug | Feature (as in data file) | New status | BifrOSt path |
|---|---|---|---|
| core-text | Writing direction and bidirectional text | partial | springboard/ui/src/bidi.rs |

Not closed: vertical text, ruby, run delegates, line and run layout objects (the paragraph model is not a typesetter),
OpenType layout, PaperKit, Core Graphics, Application Services and Translation rows for this theme.

## Notes

- `tools/parity/parity.py search` could not run in this worktree: its embeddings cache (`semantic.py` Embedder) fails
  with a bad CRC. Existing text code was found with `code_search` and grep instead. The cache needs repair in the
  parity tooling, not here.
- `tools/parity/docc.py` returns HTTP 404 for `coretext/ctrunwritingdirection`; the official pages were captured
  through REA instead.
