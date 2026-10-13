# Core Text fonts parity (slug ct-fonts)

Code: `springboard/render/src/fontinfo.rs` (table facts), `springboard/render/src/fontobj.rs` (objects, manager,
collections, cascade). SDK: `bifrost_app::fontinfo`, `bifrost_app::fontobj`. Nothing here was built or run by the
author; the lead compiles and tests. All rows still need a Midget check before they count as parity.

| Row | API (BifrOSt) | Done | Needs Midget |
|---|---|---|---|
| Font objects and descriptors | `FontSpec` (name, size, traits, weight, width, slant, matrix, variations, cascade; `attributes()`), `FontObject::{from_name, from_spec, from_data, with_size, with_matrix, with_variations, with_spec, with_family, with_traits}`, `descriptor()`, names (`family_name`, `full_name`, `postscript_name`, `display_name`, `style_name`), `symbolic_traits` | Matching ranks by weight, width, slant and trait bits; names are hard filters | Apps drawing with a matched face |
| Collections and manager | `FontManager::{register_data, register_url, unregister, unregister_url, scan_dir, available_families, available_postscript_names, scope_of, on_change, drain_events, generation, set_auto_activation}`, `FontSet`, `collection_matching`, `collection_from_specs`, `define_collection` | Scopes System/Process/Session/User; change events and listeners; auto-activation is a stored setting only (nothing fetches fonts) | Wiring the shell manager to the font library dir; persistence of User scope across sessions is not implemented |
| Character to glyph | `glyph_for_char`, `glyphs_for_str`, `with_glyph_override` (id or name), `glyph_name`, `glyph_named` | Overrides; post format 2 names and CFF names | CFF glyph names without post |
| Metrics | ascent, descent, leading, cap/x height, units per em, bounding box, underline position/thickness, slant angle, glyph count, `advances`, `bounding_rects`, `optical_bounds` (read from hhea, OS/2, post, head, hmtx, glyf) | Done for TrueType outlines | Glyph boxes for CFF faces return zero (no box reader); advances ignore variations (use `text_width`) |
| Fallback cascade | `default_cascade(lang)`, `FontObject::fallback_runs`, `render_with_fallback` | Per-language family lists ending in symbol and emoji families, then every registered face, so a glyph any installed face has avoids .notdef | The shell text path does not call this yet; the shell draws through its own face list |
| Tables | `available_tables`, `copy_table` | Done | |
| Variation axes | `variation_axes()`, `variations()` | Exposed to apps | |

Not done in this slice: shaping, paragraph styles, bidi, vertical text, ruby.
