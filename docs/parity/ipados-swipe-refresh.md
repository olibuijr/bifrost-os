# Swipe actions and pull to refresh

Status: partial. Built on parity/team-p-swipe-refresh; not yet verified on the tablet.

Matrix rows: `docs/parity-matrix/ipados/data/uikit-collections-tables.json` (Swipe actions, Refresh control).
Reference: UIKit `UIContextualAction`, `UISwipeActionsConfiguration`, `UIRefreshControl`.

## What is built

`springboard/ui/src/swipe_actions.rs` is the row swipe model:

- `ContextualAction` is one button (`id`, `title`, `style` as `ActionStyle::Normal` or `Destructive`, optional
  `background`). `id` is what the app receives back when the action runs.
- `SwipeConfig` is the actions of one edge plus `performs_first_full_swipe` (default on). The first action sits nearest
  the row edge and is the one a full swipe performs.
- `RowSwipe` holds one row. `begin_drag()`, then `drag(total_dx)` with the travel since the drag began, then `release()`.
  Release returns a `SwipeOutcome`: `Closed`, `Open(RowEdge)`, or `Performed { edge, id }`. The row rests at `settled()`
  and the view animates `offset()` to it.
- `action_at(x)` hit tests the revealed buttons in row coordinates, for taps on an open row.
- `open(edge)` and `close()` change the state without a gesture, for menu commands.

`springboard/ui/src/refresh_control.rs` is the pull to refresh model:

- `RefreshControl::observe(scroll_offset, dragging)` takes the offset a scroll view reports (a negative offset is a pull
  past the top) and whether a finger or pointer still drags it. It returns true on the call that starts a refresh.
- `progress()` (0 to 1) drives the spinner while pulling, `inset()` is the top inset to apply while refreshing.
- `begin_refreshing()` and `end_refreshing()` let the app start and finish a refresh.

## How an app uses it

Swipe actions on a row:

```rust
let mut row = RowSwipe::new(row_width,
    SwipeConfig::new(vec![ContextualAction::new(READ, "Read")]),
    SwipeConfig::new(vec![ContextualAction::destructive(DELETE, "Delete"), ContextualAction::new(FLAG, "Flag")]));
// on the drag recognizer's began event
row.begin_drag();
// on each change, with the pan translation's x since the drag began
row.drag(translation_x);
// on the release event
match row.release() {
    SwipeOutcome::Performed { id, .. } => { /* animate the row out, then run the action */ }
    _ => { /* animate to row.settled() */ }
}
```

A pointer or trackpad drag works the same way: `PointerAdapter` turns button and motion events into the touch samples the
recognizers take, so the pan translation is the same on the desktop.

Pull to refresh on a scroll view:

```rust
let mut refresh = RefreshControl::default();
// after each scroller update, with the scroller's offset and whether it is dragging
if refresh.observe(scroller.offset(), scroller.dragging()) {
    // start the load; keep the content inset at refresh.inset()
}
// when the load finishes
refresh.end_refreshing();
```

## Behaviour notes

- The swipe threshold for a full swipe is half the row width. The side of a full swipe is not resisted up to the row
  width, so the finger can reach it; other overshoot is rubber banded. A row opens when it moved past half of the
  revealed buttons and closes otherwise.
- The refresh arms at 72 points of pull (`PULL_THRESHOLD`) and starts on release. A pull back under the threshold before
  release does not refresh.
- Trackpad and wheel gestures report `dragging` while their gesture phase is active.

## Not yet done

- The SDK table and scroll views do not yet hold a `RowSwipe` per row or a `RefreshControl` per scroll view, and the
  shell does not yet draw the buttons. These are the remaining steps before the rows can be marked parity.
- Nothing has been built or run on the tablet, and the unit tests have not been run yet.
