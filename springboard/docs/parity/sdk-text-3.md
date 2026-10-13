# SDK text parity, round 3 (slug sdk-text-3)

Rows checked against `python3 tools/bifrost.py pc parity gaps` (text, fonts, attributed strings, text views) and
the current code. Several gap lines in the matrix are stale; the evidence is in the table.

| Row | API (reference) | BifrOSt path | Status this round |
|---|---|---|---|
| Core Text: font objects and descriptors | CTFont, CTFontDescriptor | `render/src/fontobj.rs` (FontSpec, FontManager, FontObject), re-exported as `bifrost_app::fontobj`; `sdk/src/font_descriptor.rs` (UserFonts) | Advanced: `UserFonts::system_object`, `mono_object`, `library_object` give the user's Settings font, or a library match, as a `FontObject` at a size. Not yet used by any app; Midget check pending. |
| Core Text: font collections and font manager | CTFontManager, CTFontCollection | `render/src/fontobj.rs` (`define_collection`, `named_collection`, `register_url`) | Already in the SDK re-export; matrix line "no font manager" is stale. No change. |
| Core Text: OpenType shaping | CTTypesetter with GSUB/GPOS | `render/src/shape_ot.rs`, `render/src/font.rs` (`measure_feat`, `render_feat`) | Matrix line "no GPOS or kern reader" is stale. No change. |
| Core Text: font metrics | CTFontGetLeading, CapHeight, XHeight, UnderlinePosition | `render/src/font.rs` (`line_metrics`), `FontObject::cap_height` and friends | Matrix line "leading, cap height, x-height, underline not read" is stale. No change. |
| Core Text: line and run layout for apps | CTLine, CTFramesetter, CTRun | `sdk/src/richlayout.rs` (`TextLayout`: `index_at_px`, `caret_px`, `selection_px`), `ui/src/typeset.rs` | Matrix line "no line layout objects for apps" is stale; `TextLayout` is public. No change. |
| Core Text: paragraph styles | CTParagraphStyle | `ui/src/typeset.rs` (ParagraphStyle), `sdk/src/richlayout.rs` (`set_paragraph_style`) | Matrix line "no paragraph styles" is stale. No change. |
| AppKit (Text): fonts, font descriptors | NSFontDescriptor | as above | Same as Core Text font objects row. |

## Still open (not closed here)

- Font fallback for characters missing from the face: `FontObject::render_with_fallback` exists in the render
  layer; the SDK's `richlayout` path still uses the `bifrost_ui::textmetrics::FontSpec` model, a second spec type
  beside `bifrost_render::fontobj::FontSpec`. Merging them is a larger refactor and was left for a later round.
- Bidirectional and vertical layout are wave-2 and wave-4 pure models, not wired into apps.

## Midget checks

- `UserFonts::system_object` and `library_object`: no app calls them yet, so nothing to see on Midget until one does.
- Core Text font rows: the matrix should be refreshed by the lead (the stale lines above).

## Evidence

- `springboard/sdk/src/lib.rs`: `pub use bifrost_render::{fontinfo, fontobj};`
- `springboard/sdk/src/richlayout.rs` `TextLayout` and `springboard/sdk/src/attributed.rs` `RichTextView::draw`.
- `springboard/render/src/font.rs` `Font::line_metrics` and the OS/2 and post reads in `Font::parse`.
