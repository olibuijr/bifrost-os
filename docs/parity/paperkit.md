# PaperKit parity: markup document model

Official reference: https://developer.apple.com/documentation/paperkit (PaperKit, markup framework, macOS 26.0 and later).

## What was built

`springboard/ui/src/markup.rs` (`bifrost_ui::markup`), a pure std-only model of a markup document:

- `MarkupDocument` mirrors `PaperMarkup` and `MarkupOrderedSet`: elements in stacking order (index 0 is the back),
  insert, remove, replace and reorder. Each element has a `MarkupId` that is never reused inside one document.
- Element kinds mirror the markup element types: `ShapeMarkup` (with `ShapeKind` Rectangle, Ellipse, Line and TextBox,
  which covers the shape and text box case), `ImageMarkup`, `LinkMarkup` and `LoupeMarkup`, held in `MarkupElement`.
- Undo and redo over edits, with a bounded history of 100 steps. A new edit clears redo. Moving an element to the
  place it already holds is not an edit.
- `encode` and `decode`: a line-based text form with a `bifrost-markup 1 <next id>` header. Text fields escape
  backslash, tab, newline and carriage return. Decoding checks each element with the same rules as an edit and
  rejects duplicate ids and ids at or above the next free id. Undo history is not saved.
- `MarkupError` mirrors `MarkupError`: unknown id, invalid frame, invalid value (by field), and decode failure with a
  1-based line number.

Frames use the crate's shared `Rect`. Colours are packed 0xAARRGGBB, as in the shell markup editor.

## Tests

`cargo test -p bifrost-ui markup` covers: id stability across remove, undo and reload; undo and redo order with
redo cleared by a new edit; reorder, clamping and no-op moves; rejection of invalid elements and unknown ids without
history; save and reload with escaped text and the next free id; and rejection of damaged saved documents with line
numbers. Per the parity task rules these tests were not run in this stage; another stage builds and tests.

## Apple names mirrored

`PaperMarkup`, `MarkupOrderedSet`, `MarkupID`, `Markup`, `ShapeMarkup`, `ImageMarkup`, `LinkMarkup`, `LoupeMarkup`,
`MarkupError`. The Rust names are `MarkupDocument`, `MarkupId`, `MarkupElement`, and the matching `*Markup` structs.

## REA evidence

- `ev_bbeabd4b1fbc4e0506ec32a3d98c44f81cb44a1ed802cb959442f60b3f460dc5`: REA passive capture of the rendered
  landing page (title "PaperKit | Apple Developer Documentation"). It lists the view controllers, configuration,
  data model, markup elements, adornments and the error enum, and states that the framework adds shapes, images and
  text boxes on top of drawing.
- DocC: `python3 -I tools/parity/docc.py paperkit --depth 1` on the same landing page, for the topic list.

## Remaining gaps

- No renderer or pointer input for the model, so no element can be drawn or created on screen.
- No app-facing SDK API. The model is not exported through `bifrost_app`.
- The shell editor and Freeform still save a flattened PNG and do not use the model or its text form.
- No redo in the shell or Freeform. No stroke selection, stylus pressure or tilt.
- No insertion menu, FeatureSet, RenderingOptions, HDR, background views, MarkupAdornment or MarkupAutoresizing.
- No MarkupInteractions for links and loupes (tap and magnification).
