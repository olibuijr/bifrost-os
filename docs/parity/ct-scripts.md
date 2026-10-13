# Text scripts, ruby, run delegates, glyph outlines (slug ct-scripts)

Host unit tests only; nothing here has been run on Midget. All app access goes through the SDK: `bifrost_app::controls`
re-exports `bifrost_ui` (`controls::bidi`, `controls::vertical_text`, `controls::ruby`, `controls::run_delegate`), and
`bifrost_app::glyph_draw` holds the outline and drawing helpers.

## Writing direction and bidirectional text
- Path: `springboard/ui/src/bidi.rs`.
- Done: full UAX 9 resolution (class table, explicit embeddings, overrides and isolates with depth limits, weak rules
  W1 to W7, paired brackets N0, neutrals N1 and N2, implicit levels), L1 per line (`Paragraph::line_levels`), L2
  reordering (`reorder_line`), L4 mirroring (`mirrored`, `display_chars`), writing direction attribute ranges
  (`Paragraph::with_attributes`, `WritingDirection`), multiple paragraphs per text.
- Pure entry points: `reorder_line(levels)`, `Paragraph::new`, `Paragraph::line_runs(range)`, `Paragraph::line_order`.
- Limits: the class table lists the common scripts, not every code point; the rest fall back on `char::is_alphabetic`.
- Midget: nothing draws through it yet until the typesetter calls it.

## Vertical text
- Path: `springboard/ui/src/vertical_text.rs`.
- Done: column layout, per-character `orientation` (upright or rotated), `vertical_form` presentation forms for CJK
  punctuation and brackets, `classify` (tate-chu-yoko for 2 to N digit runs among upright text, explicit ranges),
  `layout_text` convenience.
- Midget: glyph substitution and rotation when drawing.

## Ruby annotations
- Path: `springboard/ui/src/ruby.rs` (new). `RubyAnnotation` (position before/after/inter-character, alignment,
  overhang, size factor), `layout` (character positions, overhang, base padding), `baseline_offset`, `side_offset`.
- Midget: drawing the annotation glyphs; the attributed string does not carry a ruby attribute yet.

## Run delegates and adaptive images
- Path: `springboard/ui/src/run_delegate.rs`. Added `RunDelegate` trait, `CallbackDelegate`, `metrics_of`,
  `ImageAttachment` (variants, aspect fit, baseline shift), `placements`.

## Glyph outlines and direct glyph drawing
- Paths: `springboard/render/src/font.rs` (`Font::glyph_segments`), `springboard/sdk/src/glyph_draw.rs`
  (`glyph_path`, `text_path`, `draw_glyphs`, `fill_path`). The path type is `bifrost_ui::shape::Path` (flattened).
- Midget: visual check of `draw_glyphs` baseline placement against `Font::render`.
