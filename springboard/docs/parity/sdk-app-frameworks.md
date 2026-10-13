# SDK app frameworks parity (round 75, slice sdk-app-frameworks)

Scope: App Frameworks group, two frameworks worked deeply. Code was written without a build (the lead builds);
unit tests are added but not run. Nothing here is marked `parity` until it is checked on Midget.

Reference evidence: the matrix rows and the existing SDK modules were read from the repo. No reference
binaries or copied artwork were used for this slice (the macOS 27 extract under `~/Projects/macos-ref` was not
needed for the two rows below; the rows are behavioural models).

## AppKit (Text): text lists in attributed text

Row: `AppKit (Text)`, gap "Paragraph styles, text lists, tables and attachments in attributed text".
Paragraph styles were already present (`Attrs::paragraph`, `set_paragraph_style`), so the list item is the
part that was missing. Tables and attachments are still gaps.

- API (SDK): `bifrost_app::attributed::{ListFormat, TextList}` (re-exports of the list model in
  `springboard/ui/src/textlist.rs`), `AttributedString::set_text_list_utf16`, `AttributedString::list_markers`,
  `RichTextView::set_list`, and the `Attribute::TextList` / `AttributeKey::TextList` pair.
- BifrOSt paths:
  - `springboard/sdk/src/attributed.rs`: `Attrs::text_list` (read from the paragraph's first character),
    `list_markers` (paragraph start index plus marker text; consecutive items of one list count up from the
    starting item number, a different list or a plain paragraph restarts the count), `RichTextView::set_list`
    (undoable, covers the selected paragraphs or the caret's paragraph).
  - `springboard/sdk/src/richlayout.rs`: `paragraph_span` (shared by the paragraph style and list setters),
    `set_text_list_utf16`, list items get an 18 pt first-line and head indent in `layout_input`, and
    `TextLayout::draw` paints each marker in that gutter, right-aligned to the item, in the run's colour.
- Done (model and drawing): list formats, numbering, nesting is not modelled (the marker of an enclosing item is
  not prepended), set and clear over a selection, undo through the rich text undo manager, layout indent,
  marker drawing through the shared glyph blit.
- Not done: RTF and HTML export and import of lists (the existing exporter drops the list attribute), nested
  lists, the marker's own attributes (it takes the first character's font and colour), tables and text
  attachments.
- Midget check needed: the marker sits at the caret top of its paragraph and is drawn with the image padding
  used for glyphs; confirm the baseline alignment and the gutter width at 1x and 2x scale.
- Tests: `list_tests` in `springboard/sdk/src/attributed.rs`, `list_layout_tests` in
  `springboard/sdk/src/richlayout.rs`.

## Foundation (Fundamentals): ordered set, counted set, UUID

Row: `Foundation (Fundamentals)`, gap "IndexSet, ordered and counted sets, Decimal, UUID, NSNumber wrappers".
`IndexSet` already existed in `springboard/ui/src/indexset.rs` and is re-exported unchanged. Decimal and
the NSNumber wrappers are not done.

- API (SDK): `bifrost_app::value_types::{IndexSet, OrderedSet, CountedSet, Uuid}`.
- BifrOSt path: `springboard/sdk/src/value_types.rs` (new module, declared in `springboard/sdk/src/lib.rs`).
  - `OrderedSet<T>`: insertion order, each item once, `insert`, `remove` (order of the rest kept), `contains`,
    `index_of`, `get`, `iter`, `as_slice`.
  - `CountedSet<T>`: `add` returns the new count, `count_of`, `remove` (one occurrence), `remove_all`,
    `distinct_count`, `total`, `iter`.
  - `Uuid`: 16 bytes, `NIL`, `from_bytes`, `bytes`, `version4` (sets version and RFC 4122 variant bits),
    `new_random` (uses `entropy::bytes`), `version`, `parse` (36-character hyphenated, either case), `Display`
    in upper case with hyphens.
- Done: all of the above, with unit tests in the same file.
- Not done: Decimal, NSNumber and NSValue wrappers, key-value coding on these types, serialisation traits for
  `Uuid` (apps use `Display` and `parse`).
- Midget check: none needed for pure models; a Midget run only matters if an app adopts them.

## Not advanced this round

- `AppKit (User Interface)`: "Missing control types and bezel styles" (`springboard/sdk/src/view/forms.rs` has
  Checkbox, Stepper and PopUpButton; no combo box, colour well, date picker or level indicator).
- `AppKit (Text)`: text tables and attachments; multi-line text view wiring into apps.
- `Foundation (Files and Data Persistence)`: the matrix still lists a FileManager-style API as a gap, but
  `springboard/sdk/src/file_manager.rs` already provides it, so the row is stale and was not changed.
