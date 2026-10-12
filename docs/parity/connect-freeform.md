# Connecting Freeform to the system services (1.3.0)

Freeform (`springboard/apps/freeform-desktop`, `is.olibuijr.freeform.desktop`) now calls three shell services over the
service channel (`docs/bifrost-ui.md` "System services"): `cursor`, `pasteboard` and `screens`. The app code is in
`src/services.rs` (the glue: calls, replies and events), `src/freeform.rs` (what each tool shows and what Copy and the
export do) and `src/ink.rs` (pure PNG and factor helpers). Nothing outside the app directory changed, except that the
app now depends on `bifrost-core` (the same path dependency other apps use) to name the JSON type in the service hooks.

## What is connected

| Service | Matrix slug | Feature | What Freeform does | Calls |
|---|---|---|---|---|
| `cursor` | `appkit-user-interactions` (NSCursor, "Menus, Cursors, and the Dock") | Pointer shape per tool | Crosshair for pen, marker, rectangle and oval; poof for the eraser; arrow for the lasso and the toolbar; open hand over a selection and closed hand while it is moved | `push` on first use, `set` when the shape changes, `pop` on suspend |
| `pasteboard` | `application-services` (Pasteboard, NSPasteboard) and `core-transferable` (copy of typed items) | Copy of the selected strokes | Copy writes one item to the general board with two representations: `public.png` (the strokes cropped to their box on paper) and `is.olibuijr.freeform.strokes` (the strokes as `Drawing::to_text`) | `write` to board `general` |
| `screens` | `appkit-user-interface` (Windows, Panels, and Screens) | Backing scale for a crisp export | The main display's backing scale sets the Save to Photos factor (whole steps, 1 to 4). Above 1, the strokes are painted again at that factor, so the PNG is sharp | `list` on the first configure and when the window scale changes; `changed` event updates it |

Calls happen on real changes only: a new pointer shape (never a repeat), one Copy, one list per window scale and one
per `changed` event. Nothing calls a service per frame.

Errors: a reply with an error (for example `no service cursor` from an older shell) is written to the app log
(`freeform: cursor: ...`) and the reply is taken, so it is not kept for later. The app keeps working without the
service. At most 64 calls wait for their reply; the oldest is dropped past that.

## How a user sees it

- Hover over the board with the pen, marker or a shape tool: a crosshair. With the eraser: the poof cursor. Over the
  toolbar: the arrow.
- Lasso: the arrow, then open hands over a selected stroke and a closed hand while it moves.
- Select strokes with the lasso, then press Copy in the toolbar (second group, before Delete). A toast says
  "Copied the selection". Paste the result into another app: it arrives as a picture of the strokes. The same copy also
  carries Freeform's own data, for a later paste inside Freeform.
- Save to Photos on a sharp display writes a PNG at that display's density; the file is larger, the lines are crisp.
  On a display of the same density nothing changes.

## How to verify on the test PC (Midget)

Midget only (the owner rule; no lab VM). Freeform 1.3.0 is published with the rest of the round.

1. Publish and update: `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. Confirm the services exist: `bifrost pc midget state` lists `cursor`, `pasteboard` and `screens` in the service list
   (`system.services`).
3. Open Freeform: `bifrost pc midget open-app is.olibuijr.freeform.desktop`. Draw two strokes. Choose Lasso and loop one
   of them. Press Copy.
4. Paste check: open a text field in another app (or the shell's search field) and paste. Expect a picture or the text
   of the strokes, depending on what the general board keeps (see gaps). Check the log for `freeform: pasteboard:`
   lines; there should be none.
5. Pointer check: on the Wayland desktop path, move the pointer over the board with Pen selected and read the shape
   (crosshair). On Midget (bare KMS) the pointer is the shell's own sprite and ignores the stack, so the shape cannot be
   seen there. Check the stack instead with the cursor service's `depth` method from an app, or the log.
6. Export check: `bifrost pc midget screen /tmp/freeform.png` is the compositor, not the file. Find the saved file in
   `~/Pictures` (`Freeform-<millis>.png`) on Midget and check its size: it is the canvas size on a 1x display and twice
   that on a 2x display (`file Freeform-*.png`).
7. Light and Dark (`bifrost pc midget appearance light|dark`): the board stays paper; nothing else in the service
   behaviour depends on appearance.

Host tests, run by the lead (not built here, NO-build rule): `cargo test -p bifrost-freeform` covers
`services.rs` (pointer push, set and pop; the display list asked once per scale; the screens reply and change event;
the copy call with its two representations; a failed call), `ink.rs` (the export factor, the cropped PNG and the empty
PNG) and `freeform.rs` (copy takes only the selection, once; the Copy button follows the selection; the export size
at factors 1, 2 and 0; the pointer shape per tool, the lasso hands and the toolbar arrow). The compliance snapshot
(`tests/compliance.rs`) changes because the toolbar has a new Copy button: the lead blesses it with `BIFROST_BLESS=1`.

## Known differences and gaps

- Paste is not connected. Freeform does not read the general board back, so the stroke data it puts there is not
  used yet. A paste of strokes inside Freeform needs a read of the board and `Drawing::from_text`.
- The general board keeps items flat (one representation each, see `wire-svc-pasteboard.md`). Which of the two
  representations the shell clipboard keeps is not verified; the PNG is written first. Check on Midget (step 4).
- The copy toast says "Copied the selection" even when the pasteboard service is missing. The log says what failed.
- Pointer shape is by tool, not by position on the board edge. The shell applies the top of the stack on pointer
  motion, so a shape changes at the next move after a tool change.
- The cursor stack is process-wide. A shape pushed by Freeform stays if the app is killed without `suspend`; the
  cursor service doc lists this gap for every app.
- The SDK has no hover event, so the pointer shape is set from touch events (down, move, up). Whether the shell sends
  moves with no button held is not verified; if it does not, the shape changes on press and drag only. Confirm on the
  Wayland path.
- No cursor images: the shapes are the shell's named kinds (`cursor` service doc, "Remaining gaps").
- The export factor only changes on a display denser than the canvas. The shell renders at the scale it reports, so on
  the Midget panel the file is the canvas size; the path is there for a window on a denser output.
- The `screens` service reports one display; the factor follows the main display, not the display the window is on.
- Refresh rate, colour space and the display list's Settings UI are not part of this change (`wire-svc-screens.md`).
- Build and tests were not run here; the lead builds once.

## REA evidence (reference behaviour)

No official documentation page was read for this round. The behaviour relied on (the cursor stack, the pasteboard
items with several representations, the backing scale) is the one the service notes already cite:
`wire-svc-cursor.md`, `wire-svc-pasteboard.md` and `wire-svc-screens.md`, with their REA evidence ids.

Parity rows are not edited here (`docs/parity-matrix/` is out of scope for this round). The lead should promote the
rows above after the Midget check, following the bifrost-parity-matrix skill.
