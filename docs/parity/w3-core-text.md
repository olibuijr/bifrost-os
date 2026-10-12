# Core Text parity, wave 3

Two gap rows of the Core Text framework (`docs/parity-matrix/data/core-text.json`) get a pure Rust model each. Both
are in `springboard/render/src` and use only `std`. Neither is wired into the shell, the SDK or the text draw path
yet, so the rows move to `partial`, not `parity`.

## What was built

- **`render/src/fontregistry.rs`** (`bifrost_render::fontregistry`): a runtime font registry. `FontRegistry` holds
  families (`Family`, `Face`) registered by the caller, sorted by name and matched without regard to ASCII case.
  It supports `register` (replaces a family when its faces change, a no-op on an exact repeat), `unregister` (also
  drops the family from every collection), `family`, `family_names`, `monospaced_families`, `best_face` (upright,
  then variable, then Regular), named collections (`add_collection`, `remove_collection`, `collection`,
  `collection_names`, members kept once in canonical spelling), and `generation()`, a counter that moves on every
  change and stands in for a font-changed notification. The registry never reads files; the caller probes them,
  for example with `fontprefs::family`.
- **`render/src/kern.rs`** (`bifrost_render::kern`): the `kern` table reader. `Kern::parse` reads the version 0
  header and format 0 horizontal subtables. Subtables add up in table order, and an override subtable replaces the
  value it holds. `pair(left, right)` returns the value in font units, `adjust_px` scales it to pixels, and
  `run_adjustments` gives the extra advance after each glyph of a run. Other subtable formats, vertical and
  cross-stream subtables are skipped. The GPOS pair adjustment is not read.

Both modules have a `#[cfg(test)] mod tests` block with hand-built byte tables (kern) and hand-worked values
(registry). `render/src/lib.rs` gained one `pub mod` line per module.

## Official API names mirrored

- `CTFontCollection` (`https://developer.apple.com/documentation/coretext/ctfontcollection`): the collection
  object and its creation from a set of families. `FontRegistry` is a runtime registry with named collections,
  not a `CTFontCollection` with query and exclusion descriptors. Those are not mirrored.
- The font manager (`CTFontManager*`, including `CTFontManagerRegisterFontsForURL` and
  `CTFontManagerCopyAvailableFontFamilyNames`) is named in the matrix row. Its official page
  (`https://developer.apple.com/documentation/coretext/ctfontmanager`) returned "page not found" when fetched,
  so the registry's register and enumerate operations are matched to the matrix's description, not to that page.
- `kern` table kerning: the table layout is OpenType's `kern` table. Apple's DocC has no page for it, so it is
  not an Apple API mirror. It serves the kerning part of the `OpenType layout (kerning, ligatures, contextual forms)`
  row, which the matrix names through CTTypesetter shaping.

## REA evidence

- `ev_202d097c79c4029fc9254c833da5968332e325d3fd85ce2cc7dcaa7c5c39b7ff`: Core Text landing page
  (`https://developer.apple.com/documentation/coretext`), rendered headings include CTFontCollection and the
  layout objects.
- `ev_c75d51e75e576450e927414be4eca401c5aa11dc45e58c53f1aff4e6d05d6bf9`: CTFontCollection
  (`https://developer.apple.com/documentation/coretext/ctfontcollection`), creation and query functions.
- `ev_4a034e359eab95e45f1740cd1c6153a15cbdf5172e4658bbc23c08f28255057f`: CTFontManager page, which rendered
  as "The page you're looking for can't be found". Recorded as a failed lookup, not as evidence of the API.

## Matrix rows closed

| Framework slug | Feature (as in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| core-text | Font collections and font manager (registration, enumeration, activation) | partial | springboard/render/src/fontregistry.rs |
| core-text | OpenType layout (kerning, ligatures, contextual forms) | partial | springboard/render/src/kern.rs |

## Not done, and why

- The registry is not reachable from apps. No SDK type wraps it, and no shell code calls `register`. Auto-activation
  and the notification are a generation counter, not a callback.
- `kern` values are not used by `Font::measure` or the text draw path. Changing the advance sums would alter every
  line of text, so this needs an on-device check first. The ligature, contextual form and GPOS parts stay missing.
- Run delegates and vertical text were left alone. Adding an attachment attribute touches every match on
  `Attribute` in `sdk/src/attributed.rs`, which is too wide for a round with no build.

## Notes for the matrix lead (not edited here)

- `springboard/render/src/font.rs` now has `face_for_char` and `cascade_runs` (font fallback) and `line_metrics`
  (x-height, cap height, underline). The Core Text matrix rows still say fallback and those metrics are missing. The
  rows may be stale.
- The `OpenType layout` row names `springboard/render/src/emoji.rs` as its only shaping evidence. `kern.rs` is new
  kerning evidence once the matrix is updated.

## Verification

Not run, per the wave rules: no cargo, rustc or tests. Each new file's tests were checked by reading the code and
working the expected values by hand. `python3 tools/check-names.py` exits 0.
