# Core Text layout objects (ct-layout)

Branch work on the Core Text rows "Line and run layout objects", "Paragraph styles and tab stops", "Line breaking and
truncation", "Attributed string styling", and the layout side of "Writing direction and bidirectional text". The shaper
(kerning, ligatures) is separate work; advances come through one function so it can be swapped.

## Where things are

| API | BifrOSt path |
|---|---|
| Typesetter (create with options, suggest line break, suggest cluster break, create line) | `springboard/ui/src/typeset.rs` `Typesetter` |
| Framesetter (create frame in a path, suggest frame size with constraints, fit range) | `typeset.rs` `Framesetter`, `FramePath` (rect, ellipse, polygon) |
| Frame (lines, line origins, visible string range, path, draw) | `typeset.rs` `Frame`, `FrameLine` |
| Line (glyph count, runs, string range, typographic bounds, image bounds, pen offset for flush, offset for string index, string index for position, truncated line, justified line, draw) | `typeset.rs` `TextLine` |
| Run (glyph count, attributes, status, string indices, positions, advances, typographic bounds, draw) | `typeset.rs` `TextRun`, `RunStatus` |
| Line breaking (UAX 14 subset) and grapheme clusters (UAX 29 subset) | `springboard/ui/src/textbreak.rs` |
| Glyph advances in one place | `springboard/ui/src/textmetrics.rs` `GlyphSource`, `glyph_advances` |
| Paragraph styles | `springboard/ui/src/paragraph.rs` (`ParagraphStyle`: alignment, first-line/head/tail indents, tab stops, default tab interval, line break mode, line height multiple and limits, line spacing, paragraph spacing before and after, base direction) |
| Attributes in the attributed string | `springboard/sdk/src/attributed.rs` (`Attrs`, `Attribute`, `AttributeKey`) |
| UTF-16 ranges, layout input, glyph source over the UI font, painter, pixel queries | `springboard/sdk/src/richlayout.rs` |
| Rich text view drawing, caret, hit-testing | `springboard/sdk/src/attributed.rs` `RichTextView::{layout, draw, index_at_point, drag_selection}` |

## Per row

* Line and run layout objects: done as a model with host unit tests, and wired into `RichTextView`. Widths come from
  the UI font through `TableSource`. One glyph per grapheme cluster; glyph ids are not exposed. Image bounds are the
  typographic box (no outlines in the ui crate). Check on Midget: a `RichTextView` shows wrapped text, selection and
  caret at the right places for mixed sizes.
* Paragraph styles and tab stops: all fields are honoured by the frame layout (alignment incl. justified and natural
  for right to left, indents, tabs left/right/centre and the default interval, line height multiple and min/max, line and
  paragraph spacing). Tabs in right to left lines are measured left to right. Check on Midget: tab stops and justified text.
* Line breaking and truncation: line break opportunities for letters, numbers, punctuation, quotes, brackets, hyphens,
  ideographs, joiners, regional indicators; grapheme clusters for marks, joiners, emoji sequences, Hangul. Dictionary
  breaks (Thai) are not modelled. Truncation at the start, end and middle with a custom token, per line and by break mode.
* Attributed string styling: arbitrary font family and size, foreground and background colour, baseline offset,
  superscript and subscript, kern, tracking, stroke width and colour, underline colour and style. Ranges have `*_utf16`
  methods. A font family is stored and measured by name but drawn with the system face until the renderer can pick a
  face per run (Midget check once it can). Stroke is drawn as an offset copy, not an outline. Italic is still drawn upright.
* Writing direction: the base direction decides the paragraph level, lines are reordered with `bidi`, runs carry the
  right-to-left flag and glyphs are in visual order. Brackets are not mirrored. Not done: vertical text, ruby, run delegates.

## Adopted and removed

* `textfit` (wrap, truncate) deleted; its bench is now `ui_typeset_*`.
* `textlayout`: the line layout manager and `suggest_line_break` deleted (the storage and container model stays).
* `ui::textview::layout` (used by the Voice Memos notes view) wraps with the typesetter; its private wrapper is gone.
* `RichTextView::draw` and its private word splitting replaced by the framesetter.
* Left alone: the browser's own wrapper in `springboard/src/shell/browser/layout2/text.rs` (CSS rules, not the same
  breaking model) and the shell's `Ui::wrap` calls (plain whole-word wrap on the system font, no callers needing
  paragraph styles).
