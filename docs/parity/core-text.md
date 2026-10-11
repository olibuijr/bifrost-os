# Core Text parity (wave 2)

Three pure-Rust additions to the text path, all without new crates or unsafe code. None of them is wired into the
drawing path yet, so they are models for apps and the renderer to call, not changes in how text looks today.

## What was built

- **Font line metrics** (`springboard/render/src/font.rs`, `Font::line_metrics`, `LineMetrics`). Reads the
  x-height and cap height from the OS/2 table (version 2 and later), the line gap from hhea, and the underline
  position and thickness from post. Returns ascent, descent (positive, as the platform reports it), leading,
  cap height, x-height and underline values in px at a size. Missing tables give zero. `LineMetrics::line_height`
  returns ascent + descent + leading.
- **Font fallback runs** (`springboard/render/src/font.rs`, `face_for_char`, `cascade_runs`). Picks, per character,
  the first face in a list that has a glyph for it, and splits text into runs of one face each. Characters no face
  covers go to face 0, so they still draw as `.notdef`. A glyph 0 counts as no coverage.
- **Paragraph style model** (`springboard/ui/src/paragraph.rs`, `ParagraphStyle`, `Alignment`, `TabAlignment`,
  `TextTab`). Alignment (natural, left, right, center, justified), first-line and head indents, tail indent with
  the platform's sign rules, tab stops in any order (`next_tab`, `tab_start`), line height with multiple and
  min/max limits (`line_height`), justification spacing (`extra_per_space`) and paragraph spacing fields.

## Official API names mirrored

- `CTFont` metric getters: `CTFontGetAscent`, `CTFontGetDescent`, `CTFontGetLeading`, `CTFontGetCapHeight`,
  `CTFontGetXHeight`, `CTFontGetUnderlinePosition`, `CTFontGetUnderlineThickness`. Source:
  https://developer.apple.com/documentation/coretext/ctfont
- `CTFontCreateForString` (cascade): https://developer.apple.com/documentation/coretext/ctfontcreateforstring
- `CTParagraphStyle`, `CTParagraphStyleSpecifier`, `CTTextAlignment`, `CTTextTab`:
  https://developer.apple.com/documentation/coretext/ctparagraphstyle ,
  https://developer.apple.com/documentation/coretext/ctparagraphstylespecifier ,
  https://developer.apple.com/documentation/coretext/cttextalignment ,
  https://developer.apple.com/documentation/coretext/cttexttab

## REA evidence

- `ev_68438ea4b21e0695af8f35027e815956fa2a1517d2da6337b5e1ddc20410bf26`: Core Text landing page.
- `ev_9d57f70f0d52f0c770ec98fcec2a78ede23874028a45426a04803bac4289a19c`: CTParagraphStyle page
  (overview: alignment, tab stops, writing direction, line-breaking mode and indentation).
- `ev_1ab4282f401509aa784d8c707d4b5587237c029e44f8afa54f606c6441aea01d`: the CTFontCreateForString URL rendered
  "page not found" in REA, so it is not evidence for that API. The cascade behaviour comes from the DocC summary
  ("Returns a font reference that most accurately maps the string range based on the current font").

## Matrix rows closed

| Framework | Feature (as in data/core-text.json) | New status | BifrOSt path |
|---|---|---|---|
| Core Text | Font metrics | partial | springboard/render/src/font.rs (`line_metrics`, `LineMetrics`) |
| Core Text | Character to glyph mapping | partial | springboard/render/src/font.rs (`face_for_char`, `cascade_runs`) |
| Core Text | Paragraph styles and tab stops | partial | springboard/ui/src/paragraph.rs (`ParagraphStyle`, `TextTab`) |

All three stay partial. Metrics and the cascade are functions on `Font` and on a slice of faces; apps still get
no font object or descriptor. The paragraph model is not yet read by the attributed string drawing in
`springboard/sdk/src/attributed.rs`, and the spacing fields are stored but not applied.

## Not done

- Kerning (no GPOS or kern reader), OpenType features, bidi, vertical text and ruby remain gaps.
- The cascade is not used by `Font::measure` or `Font::render`, so missing glyphs still draw as `.notdef`.
- Nothing was built or run in this round (no cargo, by rule). Tests were written but not executed.
