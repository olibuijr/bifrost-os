# Text shaping (ct-shaping)

Module `springboard/render/src/shape_ot.rs` (`bifrost_render::shape_ot`), built on `kern.rs`.

| Area | Path | Done | Midget check |
|---|---|---|---|
| Glyph classes, mark attachment classes, mark filtering sets | `Gdef` | used by lookup flags and mark placement | marks over base in a font with GDEF |
| Substitution 1-8 | `Ctx::sub_*`, `seq_ctx`, `chain_ctx`, `reverse_sub` | all formats, nested lookups, ligature components | ligatures and calt in the system font |
| Positioning 1-9 | `Ctx::pos_*` | all formats incl. class pairs; anchors format 1-3 (device parts ignored) | kerning of "AV", accented marks |
| Legacy `kern` fallback | `OtLayout::shape_glyphs` | used when GPOS has no kern feature | n/a |
| Script, language, features, user settings | `Table::select`, `Features` | defaults ccmp locl rlig calt clig liga rclt kern mark mkmk curs dist; other tags (smcp, onum, tnum, frac, ss01-20) by user setting, value n picks alternate n | fonts that have them |
| Feature variations | `Table::variation_feature` | axis range conditions | variable system font |
| Drawing and measuring | `Font::measure_feat`, `render_feat`, `Ui::text_features`, `measure_features` | `measure_var`/`render_var` shape by default (kerning, ligatures on); marks drawn at offsets | all UI text, check width regressions |
| Attributed text | `Attrs::features`, `Attribute::Features` | per-run settings drawn by `RichTextView::draw` | rich text with liga off |

Not covered: right-to-left reordering, device and variation-index adjustments, per-range settings inside a run,
feature variations for fonts without gvar, shaping across emoji segments (default features).
Migrated: `Font::measure_var` and `render_var` no longer sum per-character advances; the per-character path remains
only for fonts with no layout tables.
