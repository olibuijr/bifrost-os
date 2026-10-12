# PaperKit parity (wave 4)

Four gap rows from the PaperKit matrix (`docs/parity-matrix/data/paperkit.json`) are now modelled in pure Rust. The
pure model is in `springboard/ui/src/markup.rs` (extended from wave 1). The app-facing API is the new
`springboard/sdk/src/markup.rs` (`bifrost_app::markup`). Nothing draws, reads pointer input or opens a window.
The shell's markup editor and its view controller are unchanged (row "Markup view controller", status `os`, is still
open: no app-facing view controller exists yet). This wave was not built or run.

## What was built

- **Insertion menu** (`bifrost_ui::markup::Insertion`): the seven menu items in menu order (`Insertion::MENU`):
  Rectangle, Ellipse, Line, Text Box, Image, Link, Loupe. `Insertion::element(origin, payload)` returns the element at
  its default size (rectangle 120 x 80, ellipse 80 x 80, line 120 x 0, text box 160 x 40, image 160 x 120, link
  120 x 20, loupe 120 x 120 at magnification 2). `payload` is the text, asset name or URL the item needs.
- **Feature set** (`FeatureSet`): a bit set of the element kinds a canvas accepts (shapes, text boxes, images, links,
  loupes). `permits(element)` tests one element. Strokes are not elements, so drawing is outside the set.
- **Auto-resizing** (`Autoresizing`, `MarkupDocument::content_extent`): `flexible_width` and `flexible_height` grow a
  canvas to the far edge of its content. A canvas never shrinks below its requested size.
- **Images placed on the canvas**: `MarkupCanvas::insert(Insertion::Image, origin, asset)` and
  `MarkupCanvas::insert_element(MarkupElement::Image(..))` in `springboard/sdk/src/markup.rs`. The document checks the
  asset name is not empty.
- **App-facing canvas** (`bifrost_app::markup::MarkupCanvas`): holds a document, a feature set and auto-resizing
  rules. Inserts are refused when the feature set does not allow the kind (`MarkupError::FeatureDisabled`). Undo and
  redo come from the document; the canvas size stays grown after undo. `save()` and `load()` round-trip the elements;
  `load` refuses elements the feature set does not allow.
- `MarkupError::FeatureDisabled(&'static str)` is the new error case; `MarkupElement::feature_name()` gives its name.

## Official API names mirrored

Source: the DocC JSON (`python3 -I tools/parity/docc.py paperkit`, `paperkit/markupautoresizing`,
`paperkit/markupeditviewcontroller`), macOS 26 and 27 pages.

- `FeatureSet`: "The features PaperKit supports in its UI and data models."
- `MarkupAutoresizing` with `flexibleWidth` and `flexibleHeight`: "Automatically adjust width/height to fit content
  changes."
- `MarkupEditViewController`, `init(supportedFeatureSet:additionalActions:)` and `supportedFeatureSet`: the insertion
  UI is limited to the feature set. The Insertion menu mirrors its insertion role.
- `ImageMarkup`, `LinkMarkup`, `LoupeMarkup`, `ShapeMarkup`: the element kinds, already modelled in wave 1.

## REA evidence

- `ev_dfe8d42dc04e6f01783ef8976c9c39e06c88fd881c46b5e5acf3ecf8fd06d1b4`: rendered page
  `https://developer.apple.com/documentation/paperkit` (run with `tools/parity/rea-page.sh`, once).

## Matrix rows closed (new status `partial`)

| Framework | Feature (as written in data/paperkit.json) | Status | BifrOSt path |
|---|---|---|---|
| PaperKit | Insertion menu for markup elements | partial | springboard/ui/src/markup.rs (`Insertion`, `Insertion::element`); springboard/sdk/src/markup.rs (`MarkupCanvas::insert`) |
| PaperKit | Images placed on the canvas | partial | springboard/ui/src/markup.rs (`ImageMarkup`); springboard/sdk/src/markup.rs (`MarkupCanvas::insert_element`) |
| PaperKit | Feature set and rendering options | partial | springboard/ui/src/markup.rs (`FeatureSet`); springboard/sdk/src/markup.rs (`MarkupCanvas::new`, `load`). Rendering options not modelled (no HDR, no background views) |
| PaperKit | Auto-resizing of markup | partial | springboard/ui/src/markup.rs (`Autoresizing`, `MarkupDocument::content_extent`); springboard/sdk/src/markup.rs (`MarkupCanvas::set_size`) |

## Tests

- `springboard/ui/src/markup.rs` `tests`: `feature_sets_combine_and_permit_elements_by_kind`,
  `insertion_menu_items_build_their_default_elements`, `content_extent_is_the_far_corner_of_all_elements`,
  `autoresizing_grows_flexible_axes_to_the_content_and_never_shrinks`,
  `insertion_items_are_checked_like_any_other_element`.
- `springboard/sdk/src/markup.rs` `tests`: six behaviour tests on feature sets, default placement, flexible width and
  height growth, undo keeping the size, save and load under a feature set, and size validation.
- Not run: this wave does not build or run tests (per the wave rules).

## Still open

- Interactive markup editor for apps (the `os` row "Markup view controller"): no app-facing controller yet.
- Rendering options and background views (the feature-set row is partial because of this).
- Adornments over markup (`MarkupAdornment`), redo in the shell editor, stroke selection and stylus input.
