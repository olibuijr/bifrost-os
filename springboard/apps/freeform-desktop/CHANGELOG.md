# Changelog

## 1.6.0

- New Pencil tool in the toolbar, beside Pen: a thinner line with pencil ink, half the pen's width. It is in the same segmented group as Pen, Marker and Eraser.
- Shadows in the Inspector are soft now: the shadow spreads a few points around its edge instead of ending in a hard copy. Opacity and offset are as before.

## 1.5.0

- New Inspector button in the toolbar opens a panel on the right for the strokes you selected with Lasso. Rectangles and ovals can be filled with a solid colour or a gradient from the current colour to the next one in the palette.
- The inspector also sets the stroke width (thin, medium, thick), a dashed outline, a soft drop shadow, and opacity (100, 50 or 25 per cent). Each change is one undo step.
- Turn, Smaller and Larger turn the selection 15 degrees or scale it to 80 or 125 per cent, around its middle. Undo puts it back.
- File > Export as PNG writes the whole board to the Photos folder at the display's sharpness, with fills, dashes, shadows and opacity as you see them. Its shortcut is Command-Shift-E.
- Stroke looks are kept for undo and Delete. Copy takes the stroke shapes only, without their looks.

## 1.4.0

- New Arrange button in the toolbar, beside Copy: with strokes selected, it lines them up in a row from the top-left corner of the selection, wrapping to a new row at the board's right edge. Each stroke keeps its shape, and Undo puts them back.
- A first-use hint explains Lasso and Arrange the first time Freeform opens. It goes away as soon as you press the board, and it does not come back.

## 1.3.0 — 2026-10-12

- New Lasso tool: drag a loop around strokes to select them, drag inside the selection to move it, and press Delete to remove it. Delete can be undone.
- New Rectangle and Oval tools: drag out a shape and it is drawn as a closed outline in the current colour.
- Every stroke is now kept as a stroke, not only as pixels, so the lasso can move and delete drawn strokes. Undo restores both the picture and its strokes.
- The toolbar shows the new tools in the same segmented group as Pen, Marker and Eraser.
- The pointer changes with the tool: a crosshair for the pen, marker, rectangle and oval, a poof for the eraser, an arrow for the lasso and the toolbar, and open and closed hands over a selection while it is moved.
- New Copy button (next to Delete, enabled with a selection): copies the selected strokes to the clipboard as a PNG, and as Freeform's own stroke data for later use.
- Save to Photos writes the picture at the display's density, so on a sharp screen the PNG is not soft.
- The cursor, pasteboard and screens services are used when the system has them. Without them the pointer stays an arrow, Copy does nothing, and the export keeps the canvas size.

## 1.2.2

- The app icon is drawn from the named system colours (Light values) and the page from `WHITE`; no literal colours left in the app.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate look: a full-size window with a unified toolbar beside the traffic lights (Pen, Marker and Eraser as one segmented group, colour wells, Undo and Clear, Save to Photos as a share button), drawn with the SDK kit in Light and Dark.
- Toolbar buttons are published as controls, so the window drags from the band around them and they can be pressed by id.
- The iPad-style 96 pt tool bar and its fixed dark colours are gone; the board fills the rest of the window.
- Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 6 padding and gap values moved to the nearest standard step (at most 2 pt).
- 2 colours now use the system colour tokens (same values).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Freeform for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Export drawings to the local account’s Pictures directory.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
