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
