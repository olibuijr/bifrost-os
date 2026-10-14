# Changelog

## 1.12.0 — 2026-10-14

- Route standard Undo, Redo, Copy and Paste through the shell's Edit commands; keep Duplicate and Delete in Arrange.

## 1.11.0 — 2026-10-14

- Text boxes, sticky notes and table cells have a caret you can move: Left and Right step through the text, Home and End go to its ends, and typing and Backspace act at the caret. The caret is drawn where it is in the wrapped text. Tab and Return start the next cell with the caret at its end. There is no selection yet.

## 1.10.0 — 2026-10-14

- Edit > Paste centres pasted strokes, notes, links and text boxes on the pointer. With the pointer over the toolbar, the paste keeps the fixed offset; Duplicate is unchanged.
- The unused second zoom, connector and search model (`canvas_view`) is removed; the app keeps one implementation of each.

## 1.9.0 — 2026-10-13

- Board items: text boxes, sticky notes, tables, links, files and pictures. The toolbar has Text, Note and Connect tools, and the Insert menu adds a text box, sticky note, table, connector, link and Picture or File. Type into an item (double tap one with the lasso, or press Return on it); Tab and Return move between table cells; Add Table Row and Column grow a table.
- Connectors join two items with an arrow and follow them when they move. Drag the Connect tool from one item to another; Delete removes a picked connector.
- Pictures and files are kept in the board's own folder, so they stay when the original moves. A double tap on a link opens it.
- Items move with the lasso, snap to the grid, scale from Arrange > Make Item Larger or Smaller, take the ink colour, and are part of undo, redo, duplicate, delete, the saved board, PNG and PDF export and printing.
- Paste puts a picture or plain text from the pasteboard on the board when it holds no strokes.
- Boards > Search Boards finds a board by title; File > Rename Board names the open one.
- View menu: Zoom In and Zoom Out (Cmd+= and Cmd+-) from 25% to 400% about the middle of the window, Actual Size (Cmd+0) and a Hand Tool. The arrow keys move the view. The grid, the PNG and PDF exports and new items follow the view.
- Freeform accepts pictures and text shared from other apps and puts them on a new board.

## 1.8.0 — 2026-10-13

- Insert menu: Line, Arrow, Triangle and Star shapes. Selection handles resize and rotate. Group, Ungroup, Lock and Unlock; align and distribute.
- Several boards: New, Duplicate, Delete, next and previous, and a Boards menu.
- View menu: dot grid and snap to grid. File: Export as PDF and Print.

## 1.7.0 — 2026-10-13

- Edit menu: Undo (Cmd+Z), Redo (Cmd+Shift+Z), Copy, Paste, Duplicate and Delete; Paste reads the strokes Copy left on the pasteboard.
- Arrange menu: Bring to Front, Send to Back and Arrange in Row.
- The board is saved to the app's data folder a moment after each edit and loaded at the next launch.

## 1.6.4 — 2026-10-13

- Shapes, transforms and fills use the shared path and scan converter; shape fills are antialiased.
- Gradient fills use the shared paint kit (`cg::paint`); the old per-app gradient model is gone.
- Strokes, dashed outlines and shape outlines draw through the shared drawing context with round caps and joins.

## 1.6.3 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 1.6.2

- The state line used by the tests is compiled only for tests; the app has no describe hook in the SDK.

## 1.6.1

- Removed the unused procedural icon module; the app uses its icon.png.

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
