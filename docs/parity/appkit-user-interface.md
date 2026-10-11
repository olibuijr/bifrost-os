# AppKit (User Interface): parity work

Framework: https://developer.apple.com/documentation/appkit
Matrix row data: `docs/parity-matrix/data/appkit-user-interface.json`

## What was built

Three pure Rust models in `springboard/ui/src` (bifrost-ui, std only, no unsafe). Each is registered with one `pub mod` line in `springboard/ui/src/lib.rs`.

1. `springboard/ui/src/screen.rs` (`Screen`, `Screens`, `Align`). Mirrors the `NSScreen` class:
   - frame and visible frame in global points, with the visible frame clipped to the screen
   - backing scale, with a non-positive value read as 1
   - `to_backing` and `from_backing` conversion of rectangles to device pixels
   - `backing_aligned` snapping with outward, inward and nearest alignment
   - `main` (chosen by the shell), `deepest` (most bits per pixel), `containing` a point, `containing_rect` (screen with the largest overlap) and `union_frame`
2. `springboard/ui/src/dock_tile.rs` (`DockTile`). Mirrors `NSDockTile` badge handling:
   - `badgeLabel` as set, with the displayed text cut to four characters (`BADGE_CHARS`)
   - `showsApplicationBadge` flag, independent of the label
   - `display()` redraw request and `take_dirty()` for the shell
3. `springboard/ui/src/cursor.rs` (`Cursor`, `CursorStack`). Mirrors the stack behaviour of `NSCursor`:
   - `push`, `pop`, `current` (arrow when empty) and `set` (replaces the top)
   - `hide` and `unhide`, where each `unhide` negates one earlier `hide`

Unit tests: 10 in `screen.rs`, 6 in `dock_tile.rs`, 5 in `cursor.rs`.

## Official API names mirrored

- NSScreen: `main`, `screens`, `deepest`, `frame`, `visibleFrame`, `backingScaleFactor`, `depth`, `convertRectToBacking`, `convertRectFromBacking`, `backingAlignedRect(_:options:)`, `containing point` (lookup has no single Apple call; it follows the global-coordinate model of the frame).
- NSDockTile: `badgeLabel`, `showsApplicationBadge`, `display()`.
- NSCursor: `current`, `arrow`, `push()`, `pop()`, `set()`, `hide()`, `unhide()`.

Official doc topics were read with `tools/parity/docc.py` for `appkit/nsscreen`, `appkit/nsdocktile` and `appkit/nscursor`.

## REA evidence

- `ev_ec5ff1350de1641a467b991d69b4d860787a5eb12673ccc3f9f7bde6efd81272`: rendered official page for NSScreen (`https://developer.apple.com/documentation/appkit/nsscreen`), via `tools/parity/rea-page.sh`.
- `ev_f82ebd8719732e253617f0fa4a235954eb9b61fb56c2c20aeaeb04b009140b94`: framework landing page, recorded in the matrix row before this work.
- NSDockTile and NSCursor: no REA page evidence was taken in this round. Their API shape comes from `docc.py` only.

## Decisions and limits

- Screen rectangles use global points. `to_backing` subtracts the screen origin, so pixel values are relative to the screen, as the `convertRect` names describe.
- The alignment options are a three-value enum, not Apple's full option set.
- `deepest` uses bits per pixel. Apple's colour-space and gamut checks are not modelled.
- The Dock tile model stores the label and the redraw flag. It does not model the tile size, the tile content view or a Dock menu.
- The cursor model stores shape names only. Cursor images and hot spots are not modelled.
- The models are not yet exposed through `bifrost_app` (the SDK). The shell does not feed display geometry to apps, and it does not draw cursors from the stack. Existing `Context::set_badge` in `springboard/sdk/src/runtime.rs` still sends the label straight to the shell, so the new `DockTile` model is not yet used by the shell.

## Remaining gaps

- App-facing sound, speech synthesis and haptics API (system layer `audiod` exists, no SDK API).
- SDK wiring of screens: the shell must report displays to apps.
- Cursor images with hot spots, drawn by the shell from the cursor stack.
- Dock tile menus and drawing of the app-icon badge.
- Full accessibility roles and rotors.
- Missing control types and bezel styles.

## Build and test status

Not built or run in this round, as the task required. The tests are written against the existing `Rect` helpers (`intersection`, `union`, `contains`). A build and test pass is still needed.
