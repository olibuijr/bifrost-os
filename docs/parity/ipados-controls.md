# Tablet views and controls parity (picker, content unavailable, large content)

Three gap rows of the tablet platform's Views and controls section (`docs/parity-matrix/ipados/data/uikit-views-controls.json`)
are now partial: a wheel picker, a content-unavailable view and a large content viewer for bar items. Each has a pure model
in `springboard/ui` and a view in `springboard/sdk/src/view`. The models hold the state and the arithmetic; the views map
touches and keys onto them and draw. Everything here was built on the `parity/team-p-controls` branch and is not yet
verified on the tablet.

## What was built

### Wheel picker (`UIPickerView`)

- `springboard/ui/src/picker_wheel.rs` (`pub mod picker_wheel;` in `springboard/ui/src/lib.rs`). `PickerDataSource` is the
  data source: `components`, `rows`, `title` and an optional `width` (0 shares the width evenly). `PickerWheel` holds one
  `WheelColumn` per component. A column keeps its row count, its selected row (`selected`, `None` when it has no rows) and
  its position in rows. `drag` follows the finger and rubber-bands past the first and last rows. `release` projects the
  velocity over `FLING_PROJECTION_S` and settles on the nearest row; `settle_to` settles on a given row (used for taps).
  `tick` eases the settle over `SETTLE_S`. `step` is the keyboard step; `visible_rows` gives the rows to draw.
  `reload_all` and `reload_component` keep selections that still exist.
- `springboard/sdk/src/view/picker.rs` (`PickerView`). Build it with `PickerView::new(Box::new(source), "action")` and
  optionally `.on_select(|component, row| ...)`. A drag scrolls the column under the finger; a flick carries it further;
  a tap on a row settles on that row; a cancelled touch snaps without a flick. Arrow keys step the focused component and
  Left and Right move the focus between components. A user change sends the action up the responder chain, then calls the
  handler. `select_row` and `reload_all` do not send anything, as `selectRow(_:inComponent:animated:)` does not call the
  delegate on the reference platform.

### Content unavailable view (`UIContentUnavailableView`)

- `springboard/ui/src/content_unavailable.rs`. `ContentUnavailable` is a plain configuration: `image` (a symbol name),
  `text`, `secondary_text`, `buttons` (`ContentButton { title, action }`) and `axis`. The presets are
  `ContentUnavailable::empty()` (tray symbol, "No Content") and `ContentUnavailable::search()` (magnifier symbol,
  "No Results", "Check the spelling or try a new search."). `accessibility_label` joins the title and secondary text.
- `springboard/sdk/src/view/content_unavailable.rs` (`ContentUnavailableView`). `ContentUnavailableView::new(config)`
  stacks the title, secondary text and buttons in the middle of its frame. `ContentUnavailableView::with_image(config,
  image)` also draws an `Image` above the title at its pixel size; the symbol name in the configuration is for the app to
  render. The labels and buttons are the accessibility elements.

### Large content viewer (`UILargeContentViewerInteraction`)

- `springboard/ui/src/large_content.rs`. `LargeContentItem` holds the title, the optional symbol and `shows`
  (`showsLargeContentViewer`). `LargeContentInteraction` is the hold state machine: `press` starts a hold, `moved` cancels
  it when the finger leaves a radius of `SLOP_PT`, `tick` presents the viewer after `HOLD_S` and `end` dismisses it. A
  disabled interaction never presses.
- `springboard/sdk/src/view/large_content.rs` (`LargeContentView`). It wraps the item's view:
  `LargeContentView::new(item, Box::new(button), enabled)`, optionally `.with_image(image)`. A held press shows the title,
  and the picture, in a bubble above the item (below it when there is no room above). Its intrinsic and layout metrics come
  from the wrapped view, so it can sit in a stack like the item does.
- `springboard/sdk/src/view/mod.rs` gained the observer hook that makes this possible: `View::observes_pointer` and
  `View::observe_pointer`. The window offers each pointer event to the views on its path that observe pointers, outermost
  first, before the normal dispatch. The item still receives every event, so the wrapper never takes a touch away.

## How an app uses them

```rust
use bifrost_ui::picker_wheel::PickerDataSource;
use bifrost_ui::content_unavailable::ContentUnavailable;
use bifrost_ui::large_content::LargeContentItem;

// Picker: a date-like list with two components.
let picker = PickerView::new(Box::new(MySource), "pickerChanged")
    .on_select(|component, row| log::info!("component {component} -> row {row}"));
let month = picker.selected_row(0);   // read the selection any time

// Empty state for a list that has no items.
let empty = ContentUnavailableView::new(ContentUnavailable::empty().with_button("New Note", "newNote"));

// Large content viewer around a toolbar button, enabled for large text sizes.
let share = LargeContentView::new(LargeContentItem::new("Share"), Box::new(Button::new("Share", "share")), text_is_large);
```

The app keeps the `PickerView` and reads the selection from it when its action arrives; the action is the change
notification, and the handler is the delegate method.

## Limits

- A long press on a button still fires the button's action on release, because the item keeps every event. An app that
  wants the press to be only a preview should give the bar item an action it can ignore while the viewer is showing.
- The large content viewer is enabled by the app (`set_enabled`); it is not switched on automatically at accessibility
  text sizes, because the content size categories are not modelled yet (only the continuous text scale exists).
- The viewer's bubble is positioned inside the window's width only on the left edge; a bar item at the right edge gets
  a bubble that may run past the screen. Fixing that needs the window size in the view's draw pass.
- The picker has no date or time variants, no accessibility value adjustment (`ax_set_value`) and draws five rows.
- Neither the picker nor the content view is checked on the tablet's touch panel; the 36 pt row height and the 4 pt
  corners are BifrOSt's own values, not the reference platform's.

## Tests

Unit tests cover the models: `picker_wheel` (selection, reload, drag and rubber band, flick and settle, keyboard steps,
data source, widths), `content_unavailable` (presets, builders, accessibility label) and `large_content` (hold, slop,
disabled, dismiss). The SDK views have tests for the picker's keys, actions, handler, drag, tap and cancel, and for the
large content view's hold and dismiss through the observer hook. These are not run on the lab machine in this change.
