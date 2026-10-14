# Pointer lock, band selection and focus halo

Three UIKit pointer and focus features from the reference platform's "Pointer interactions" and "Focus-based
navigation" sections, built as pure models in the UI toolkit with a small SDK surface. Status is `partial` in
`docs/parity-matrix/ipados/data/uikit-pointer-menus-focus.json`: the models and their tests exist, but the shell does
not yet route the events, and nothing has been run on the tablet or on Midget.

## What is built

| Feature | Reference name | Model | SDK surface |
|---|---|---|---|
| Pointer lock | `UIPointerLockState` | `springboard/ui/src/pointer_lock.rs` | `bifrost_app::pointer_focus::pointer_lock` |
| Band selection | `UIBandSelectionInteraction` | `springboard/ui/src/band_select.rs` | `bifrost_app::pointer_focus::band_select` |
| Focus halo | `UIFocusHaloEffect` | `springboard/ui/src/focus_halo.rs` | `bifrost_app::pointer_focus::focus_halo` |

`springboard/sdk/src/pointer_focus.rs` re-exports the three modules. Each model is independent of the others and of
the shell; they take plain numbers and rectangles, so the same code runs on a desktop with a mouse and a keyboard and
on the tablet with touch.

### Pointer lock

`PointerLock` keeps the lock state of the scenes. Only the key scene can hold the lock, and at most one scene holds it.

- `set_key_scene(scene)` is called when the key scene changes. A lock held by a scene that stops being key is released,
  and the call returns `Change::Released`.
- `request(scene)` returns `Err(LockError::NotKeyScene)` for any other scene, and `Ok(Some(Change::Acquired))` when
  the lock is granted. Asking again while the lock is held does nothing.
- `release(scene)` returns the change only when that scene held the lock.
- `cursor_hidden()` is true while the key scene holds the lock: the shell hides the cursor.
- `motion(dx, dy)` returns the relative delta to deliver to the locked scene, or `None` when the cursor should move by
  its absolute position.

### Band selection

`BandSelection::begin(x, y, base, extend)` starts a drag on empty space. `base` is the selection before the drag, and
`extend` is set when Shift or Command is held. Call `update(x, y)` for each pointer or finger position. The band becomes
active after the pointer moves `START_DISTANCE_PT` (6 pt), so a tap stays a tap. `rect()` gives the normalised band to
draw, and `selected(items)` gives the ids it touches, sorted, without repeats. `finish(items)` ends the drag and returns
the final selection. A tap on empty space clears the selection unless `extend` is set.

Items are `Item { id, frame }`. Touching edges do not count as overlap.

### Focus halo

`halo(frame, corner_radius)` returns a `Halo` with an inner edge and an outer edge, each with its own radius. The ring is
`HALO_GAP_PT` (2 pt) outside the view and `HALO_WIDTH_PT` (3 pt) wide. Corner radii follow the view's corners and are
capped at `MAX_RADIUS_PT` (6 pt), in line with the flat-corner rule. `halo` returns `None` for an empty frame.
`in_ring(halo, x, y)` tests a point against the ring.

## How an app uses it

```rust
use bifrost_app::pointer_focus::{band_select::{BandSelection, Item}, focus_halo, pointer_lock::PointerLock};

// Band selection in a collection: start on a press over empty space.
let mut band = BandSelection::begin(x, y, selected.clone(), shift_held);
// On each move:
if let Some(rect) = band.update(px, py) {
    draw_band(rect);
}
// On release:
let picked = band.finish(&items);

// Focus halo around the keyboard-focused control:
if let Some(h) = focus_halo::halo(frame, corner_radius) {
    draw_ring(h.outer, h.outer_radius, h.inner, h.inner_radius);
}
```

A scene that needs relative motion (a drawing surface or a game) asks for the lock while it is key and reads
`motion(dx, dy)` for its deltas. The app does not own a `PointerLock`: the shell owns one and reports the key scene.

## What is missing

- The shell does not yet hold a `PointerLock`, hide the cursor from it, or send relative motion to a locked scene.
- The shell does not start a band from a press on empty space in a collection view.
- The SDK focus ring in `springboard/sdk/src/view/controls.rs` (`focus_ring`) still draws its own ring; it does not call
  `focus_halo::halo` yet.
- Focus groups and a focus environment tree are not modelled.
- None of this has been built, tested on the host, or run on the tablet. The unit tests in each module are the only
  checks so far.

Built on parity/team-p-pointer-focus; not yet verified on the tablet.
