# Text formatting panel and formatting coordinator

Parity row: "Text formatting controller" (UITextFormattingViewController, UITextFormattingCoordinator) in
`docs/parity-matrix/ipados/data/uikit-text-display-textkit.json`. Status: partial, built on `parity/team-p-text-formatting`,
not yet verified on the tablet.

The handwriting (scribble) row of the same file is n/a: pen input needs a stylus, and the tablet has none.

## What is built

The code is in `springboard/sdk/src/text_formatting.rs`. It works on the SDK's rich text view (`RichTextView` in
`springboard/sdk/src/attributed.rs`) and has no drawing of its own.

- `Mark` names the four character styles the panel toggles: bold, italic, underline and strikethrough.
- `FormatCommand` is one change the panel can ask for:
  - `Toggle(Mark)` turns a mark off when every character of a non-empty selection already has it, and on otherwise.
  - `Font(Option<String>)` sets the font family; `None` is the system font.
  - `Size(SizeClass)` sets the text style size.
  - `Color(Option<ColorToken>)` sets the text colour; `None` is the default colour.
  - `Alignment(Alignment)` sets the alignment of every paragraph the selection touches.
  - `List(Option<ListFormat>)` makes the touched paragraphs items of a list; `None` ends the list.
- `apply(view, command)` runs a command on a `RichTextView`. Character commands act on the selection, or set the typing
  attributes when the selection is a caret, so the next typed character takes them. Each command is one undo step of the
  view, and `RichTextView::key_command` with `Cmd+Z` undoes it.
- `state(&RichText)` reads the formatting in effect: a caret reads the typing attributes, and a selection reads its first
  character for values. A mark counts as on only when the whole selection has it, so a mixed selection shows the mark off.
- `panel_items(&FormatState)` lists the panel controls in display order (Font, Size, Bold, Italic, Underline,
  Strikethrough, Color, Alignment, Lists) with an `active` flag that is set when the value is not the default.
- `FormattingCoordinator` binds the panel to one text view by a `ViewId`. `bind` is called when a view gains focus,
  `unbind` when it loses focus (a view that is not bound leaves the binding alone), and `route` applies a command only to the
  bound view. A command for any other view returns false and changes nothing.
- `FormattingCoordinator::key_command` maps `Cmd+Shift+X` to strikethrough for the bound view.

The unit tests in the same file cover toggling with mixed and caret selections, font, size and colour at a caret and over a
selection, alignment across paragraphs, lists, undo, routing to the bound view, unbinding, the shortcut and the panel items.

## How an app uses it

An app that edits rich text holds a `RichTextView` and a `FormattingCoordinator`:

```rust
use bifrost_app::text_formatting::{apply, panel_items, state, FormatCommand, FormattingCoordinator, Mark};

// When the text view gains focus, bind the panel to it. The view id is any stable id the app gives the view.
coordinator.bind(text_view_id);

// Draw the panel from the state of the focused view.
let shown = panel_items(&state(&view.text));

// A tap on the bold control (pointer, touch or keyboard all end here):
coordinator.route(text_view_id, &mut view, &FormatCommand::Toggle(Mark::Bold));

// When the text view loses focus:
coordinator.unbind(text_view_id);
```

A panel that changes a value (the font menu, the size menu, the colour well) builds the command from the value the person
picked and routes it the same way. The panel itself does not hold state: it reads `state` each time it draws, so it
always shows the selection.

## Desktop and tablet

The model has no form-factor code. A pointer, a touch or a key press produces the same `FormatCommand`, so the panel
behaves the same on a desktop and on the tablet. The desktop shortcuts `Cmd+B`, `Cmd+I` and `Cmd+U` remain on
`RichTextView::key_command`, and `Cmd+Shift+X` is on the coordinator.

## Not done

- The shell does not draw the panel, and it does not yet bind the coordinator to the focused text view. The touch text
  editor in `springboard/src/shell/textarea.rs` is not a `RichTextView`.
- Font choices come from the caller. The panel does not list the system fonts.
- Sizes are the Golden Gate text styles only. Per-font point sizes are not in the panel.
- Nothing has run on the tablet. The row stays partial until a tablet run confirms the panel and its commands.
