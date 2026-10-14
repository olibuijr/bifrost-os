# AppKit (User Interactions) parity: round 1

Framework status stays `partial`. This round adds three pure Rust models to `bifrost_ui` (std only, no new crates,
no unsafe). None of them is wired into the compositor or the SDK yet, so the matrix does not move any row to `parity`.

Official reference: https://developer.apple.com/documentation/appkit (the User Interactions topics: Mouse, Keyboard,
and Trackpad; Menus, Cursors, and the Dock; Gestures; Drag and Drop; Accessibility).

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/tracking.rs` | `NSTrackingArea` (`init(rect:options:owner:userInfo:)`, options for mouse entered/exited and mouse moved) | `Tracker` keeps the set of areas the pointer is inside. `pointer(x, y)` returns exits, then enters, then moves, each area reported on its own. `pointer_left()` exits everything. `update(areas)` exits an area that disappears while hovered and does not re-enter areas that stay. Rectangles are half-open, as `Rect::contains` already was. |
| `springboard/ui/src/cursor.rs` | `NSCursor` (`push`, `pop`, `set`, `hide`, `unhide`, `setHiddenUntilMouseMoves(_:)`, `current`, system cursor names) | `Cursors` is a stack of `Kind` shapes with a top-of-stack current shape, a nested hide count and a hidden-until-moved flag. `Kind::shape()` gives the cursor-shape protocol name (`text`, `ew_resize`, ...). `set` replaces the top shape. |
| `springboard/ui/src/gesture.rs` (`Magnify`, `Rotate`) | `NSMagnificationGestureRecognizer` (`magnification`), `NSRotationGestureRecognizer` (`rotation`, `rotationInDegrees`) | Two-finger recognizers fed with both finger positions per frame. `Magnify` reports spacing against the spacing at landing. `Rotate` unwraps the angle at half a turn and reports radians and degrees. Both use the existing `SLOP` and `State` types. |

Each module has its own `#[cfg(test)] mod tests` (tracking 6 tests, cursor 5, gesture 6 new tests).
`springboard/ui/src/lib.rs` gained `pub mod tracking;` and `pub mod cursor;`.

## Official API evidence

Read with `python3 -I tools/parity/docc.py` (DocC JSON) on 2026-10-11:

- `appkit/nscursor`: `push()`, `pop()`, `set()`, `hide()`, `unhide()`, `setHiddenUntilMouseMoves(_:)`, `current`, the
  system cursor accessors (`arrow`, `iBeam`, `pointingHand`, `resizeLeftRight`, ...) and the deprecated
  `mouseEntered(with:)` / `mouseExited(with:)` cursor-rectangle callbacks (not mirrored: deprecated).
- `appkit/nstrackingarea`: an area that generates mouse-tracking and cursor-update events; options combine per area.
- `appkit/nsmagnificationgesturerecognizer`: "a continuous gesture recognizer that tracks a pinch gesture that magnifies
  content", property `magnification`.
- `appkit/nsrotationgesturerecognizer`: "two trackpad touches moving opposite each other in a circular motion",
  properties `rotation` (radians) and `rotationInDegrees`.

REA evidence (`tools/parity/rea-page.sh`, passive rendered-page capture):

- `ev_89ff57d02f5e0c06fb16d7ed2cd82afa0cc42146fc5d9de9210aefc406081fb0`: AppKit landing page. Confirms the
  User Interactions topic group (Mouse, Keyboard, and Trackpad; Menus, Cursors, and the Dock; Gestures; Drag and
  Drop; Accessibility for App Kit) and the Touch Bar topic.
- `ev_c117807174d13fdfac5be6ad1bb26c6614adc9e640b17cb09120c645e24de789`: the evidence already recorded in the matrix
  row for this framework (kept as `rea_evidence`).
- A first REA run against `.../documentation/appkit/user-interaction` returned Apple's "page you're looking for can't
  be found" page. That URL is wrong; the landing page above is the right one. It is not recorded as evidence.

## Decisions

- Rotation sign: positive is the direction `atan2` measures in screen coordinates (y down), which is clockwise on
  screen. The official docs do not state a sign, so the model documents its own convention. Check it against the
  compositor's touch axes when the input is wired.
- Magnification is `spacing / base - 1`, so 0.0 is no change. This matches the "amount of magnification" wording and
  makes a pinch that starts from zero spacing fail instead of dividing by zero.
- Tracking events are per area and per pointer position. A `Moved` event fires for every position report inside a
  `MOVED` area, including repeated positions. The caller decides whether to coalesce.
- Cursor hide is counted and nested, matching the docs' "negates an earlier call to hide". `unhide` below zero does
  nothing.

## Remaining gaps

- Scroll wheel events do not reach apps. The app input path is still only Down, Move, Up and Cancel.
- Tracking areas are not fed by the app input path, and views have no tracking-area API.
- Cursors: no SDK call sets the cursor, and the compositor does not yet receive `Kind::shape()` requests.
- Trackpad magnify and rotate: the compositor does not send two-finger touches to apps.
- Dock tile custom content and the Dock menu are missing. Touch Bar is Apple-only hardware (`n/a`).
- Drag and drop inside the retained view kit, and nested accessibility tree nodes, are unchanged from the matrix.

## Verification

Not built or run in this round, by instruction. A later stage builds and tests `bifrost_ui`. Name-guard check:
`python3 tools/check-names.py` (see the commit for the result).

# AppKit (User Interactions) parity: round 2 (team k)

Framework status stays `partial`. Round 2 reconciles the matrix row with the code that already exists, and adds one
pure model. Host tests only; not built or run on Midget in this round.

## Reconciled with the code

The round 1 gaps were partly stale, and the matrix row still listed them as missing:

- Scroll wheel: `springboard/ui/src/wheel.rs` (`Sequencer`, phases and momentum). Not delivered to apps.
- Tracking areas: `springboard/ui/src/tracking.rs` (`Tracker`, options, key window and visible rect). Not fed by the app input path.
- Drag sessions: `springboard/ui/src/drag_session.rs` is used by the SDK transfer path (`springboard/sdk/src/transfer_drag.rs`).
- Cursor shapes: the SDK `springboard/sdk/src/cursor_service.rs` pushes `Kind` shapes to the shell.
- Dock menu and progress: `springboard/sdk/src/dock_tile.rs` (`set_menu`, `set_progress`, `bounce`), rendered by the shell
  in `springboard/src/shell/desktop_bar.rs` (`Menu::Dock`). The Dock menu is no longer a gap.
- Nested accessibility: `springboard/ui/src/ax.rs` records parent links (`AxLink`, `children_of`, `parent_of`,
  `index_in_parent`). The AT-SPI bridge does not read them yet.

## What was built

| Module | Mirrors | What it does |
|---|---|---|
| `springboard/ui/src/trackpad_gesture.rs` | Two-finger magnify and rotate events (`NSEvent` magnify and rotate, the recognizers in `gesture.rs`) | `TwoFinger::frame(a, b)` runs `Magnify` and `Rotate` on the same two-finger touch and returns `Event { kind, state, value }`. Only `Began`, `Changed` and `Ended` are reported; magnify is reported before rotate in a frame. `lift()` ends the gestures that began and resets both recognizers. Six unit tests. |

`springboard/ui/src/lib.rs` gained `pub mod trackpad_gesture;`.

## Official API evidence

- `appkit/nsmagnificationgesturerecognizer` and `appkit/nsrotationgesturerecognizer`, as recorded in round 1.
- Event delivery for these gestures is described under the AppKit User Interactions topics (Gestures), read from the
  landing page evidence `ev_89ff57d02f5e0c06fb16d7ed2cd82afa0cc42146fc5d9de9210aefc406081fb0`.

## Remaining gaps

- The app input path carries only Down, Move, Up and Cancel. Scroll wheel, tracking-area (hover) and the new two-finger
  events need a `ToApp` variant in the shell protocol before apps receive them.
- Cursor art (`cursor_art.rs`) is not yet drawn by the compositor.
- Dock tile content beyond the progress bar (a custom `contentView`).
- Drag and drop inside the retained view kit.
- Nested accessibility nodes in the AT-SPI bridge.
- Touch Bar is Apple-only hardware (`n/a`).

## Verification

Not built or run, by team rule (agents write and commit code; the lead builds and tests). The matrix row for this
framework is now `partial` with coverage 50.0%, down from 56.2%. The Mouse, Keyboard, and Trackpad row was marked
`parity`, but wheel, hover and two-finger events do not reach apps, so it is `partial` until they do.
