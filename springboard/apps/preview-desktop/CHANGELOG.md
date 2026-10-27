# Changelog

## 0.11.18

- Rebuilt for 0.10.44: every touch says whether a mouse or a finger made it, so a mouse drag no longer scrolls lists on the desktop (the wheel, trackpad and scroll bar still do).

## 0.11.17

- No toolbar tool looks selected while no document is open.

## 0.11.16

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.11.15

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.11.14

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.11.13

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.11.12 — 2026-10-23

- Opens images and PDFs from Files; PDFs with the standard non-embedded fonts render their text.

## 0.11.11

- Fix: with the sidebar hidden or no document open, the toolbar starts after the traffic lights instead of under them.

## 0.11.10

- Fix: the Viewer starts again. It no longer declares the `screen` permission (the shell refuses it for every app but
  one); File > Take Screenshot goes through the shell's screenshot service, which needs only the person's consent.

## 0.11.9

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.11.8 — 2026-10-17

- Services menu: "Open Images in Viewer" appears for a picture or PDF selection (the app declares it as a `[[service]]`
  in its manifest) and opens the first picture or PDF of the selection in the Viewer.

## 0.11.7 — 2026-10-17

- Preview > Speech > Start Speaking reads the selected PDF text, or the shown page's text when nothing is selected,
  through the shell's speech service; Stop Speaking ends it. Pictures have no text to speak.
- File > Take Screenshot > From Selection…, From Window… and From Entire Screen ask the shell's new screenshot service
  (needs the Screen permission, added to the manifest). The picture is saved to Photos and opens in the viewer when the
  file is there.

## 0.11.6 — 2026-10-17

- Pages > Insert > Page from File… merges every page of another PDF after the shown page, and Pages > Insert > Blank
  Page adds a blank page the size of the shown page. The result is written to a copy under the app's data folder and
  opened; the other file's objects are renumbered and the page tree is rewritten. Undo does not reach back past an insert.
- Fill PDF forms: text fields and check boxes are read when a PDF opens, drawn over the page, typed into (Return or
  Escape leaves a field) or toggled by a click, and saved by Save, Export PDF and Print. Radio groups, choice lists and
  signature fields are not filled yet.

## 0.11.5 — 2026-10-14

- Outline, markup, bookmark and search sidebar rows use the Golden Gate source-list widget and its hit model; page
  thumbnails remain a content-specific gallery. Inspector metadata scan bounds are named by their purpose. Markup
  colours use named system tokens, with highlight opacity applied separately; black remains reserved for redaction.

## 0.11.4 — 2026-10-14

- Scale the magnifier handle to the shared legacy-control radius.
- Viewer canvas and sidebar backgrounds follow the active appearance; the named document render bounds describe
  off-screen page rendering rather than the app window. PDF work runs on a serial Utility dispatch queue rather than a
  persistent blocking receiver thread. PDF paper placeholders and opaque picture redaction remain intentionally white
  and black, respectively.

## 0.11.3 — 2026-10-14

- Resolve Preview sidebars, controls and placeholders with appearance-aware semantic color pairs.

## 0.11.2 — 2026-10-14

- Move Preview's document-editing commands into its own app menu instead of duplicating the shell Edit menu.

## 0.11.1 — 2026-10-14

- Add the generated app compliance harness.
## 0.11.0

- The Inspector lists each piece of markup on its own row, with its kind, its page and the text of a note or text box,
  under the annotation counts. The rows come from `inspector::annotation_rows`, which is tested.

## 0.9.0

- File > Save (Cmd+S) writes the markup, page order and turns into the document and opens it again. A PDF is written
  with its markup as annotations; a picture is written in its own format. File > Revert to Saved opens the file again
  and drops every change since it was saved. The copy the window keeps for a file chosen in the open panel is the one
  Save replaces.
- Edit > Copy (Cmd+C) puts the text of a PDF page on the clipboard, or only the lines inside the selection when there
  is one; a picture goes to the clipboard as PNG. Edit > Select All (Cmd+A) selects the shown page. Copy works by
  line, not by word: the extraction does not give the position of each word.
- Tools > Text Box places a text box at a click and takes the typed text; it is saved into the PDF as a FreeText
  annotation in Helvetica. Text boxes are not burned into exported pictures yet.
- Tools > Star, Polygon and Speech Bubble draw closed shapes. They are saved into the PDF as polygon annotations and
  burn into exports.
- Tools > Redact draws a black box over a picture; the box is burned into saved and exported pictures. It is not offered
  for a PDF, whose text under a box would stay.
- Tools > Save Signature keeps the strokes drawn on the shown page (Sign tool) in a library of up to nine.
  Tools > Insert Signature places one in the middle of the page as drawn strokes, at one undo step.
- View > Hide Toolbar (Cmd+Option+T) hides the toolbar and gives its height to the page.
- Adjust > Show Adjust Color shows sliders for exposure, contrast, saturation and temperature on the right side. A drag
  is one undo step. Sharpness has no filter yet.
- Settings menu: Remember Last Page reopens a document on the page it was last shown on; Open PDFs as Contact Sheet
  opens them as the grid; Annotation Author sets the author written into new markup (typed in the status line).

## 0.8.0

- Tools > Select drags out a rectangle on the page. Tools > Crop (Cmd+K) keeps the selection of a picture and moves its
  markup with it; View > Zoom to Selection fills the window with it.
- Tools > Flip Horizontal and Flip Vertical mirror a picture and its markup. Tools > Instant Alpha clears the region of
  one colour around a click to transparent, and Tools > Remove Background clears the background that reaches the
  picture's edge. All of these can be undone, and Revert to Original brings the pixels back. PNG export now keeps
  transparency; JPEG export lays a transparent picture over white.
- New markup: Oval, Line, Arrow, Underline and Strike Through. They draw in the window, burn into exports and are saved
  into the PDF as Circle, Line, Underline and StrikeOut annotations.
- Edit > Find (Cmd+F) searches the text of a PDF: type the text in the status line and press Return. The matching lines
  are listed in the sidebar (View > Search Results) and Find Next and Find Previous (Cmd+G, Shift+Cmd+G) step through
  the pages that hold them.
- View > Markup and View > Bookmarks list every mark and bookmark in the sidebar; a click goes to its page.
- View > Contact Sheet (Cmd+2) shows every page in a grid. View > Slideshow (Shift+Cmd+F) shows the pages one after
  another without the toolbar and sidebar; arrows, space and Return step, Escape ends it.
- Tools > Show Inspector (Cmd+I) lists the file's name, kind, size, pages or pixels, markup counts and, for a JPEG, its
  camera, exposure and location. Tools > Show Location Info shows the place a picture was taken. Tools > Show Magnifier
  enlarges the picture under the pointer.
- File > New from Clipboard (Cmd+N) opens a picture from the clipboard.
- Tools > Adjust Size (Cmd+Option+I) resamples a picture: type a width, a width x height or a percentage in the status
  line and press Return. File > JPEG Quality chooses High, Medium or Low for exported JPEGs.

## 0.10.0

- File > Export As (Cmd+Shift+S) shows an export panel on the right: PNG, JPEG or PDF, the JPEG quality, the export size
  (25% to 200%) and an Export button. The quality applies to JPEG and the size to the formats that scale. A PDF's password
  is not set from the panel yet.
- Opening a password-protected PDF asks for its password in the status line. The typing shows as stars, Return opens the
  file and Escape leaves it closed. A password is kept for the session, so Revert to Saved does not ask again. Owner
  passwords are not tried, and a PDF with a password cannot be saved with its markup yet.
- Adjust > Sharpen and Soften (and a Sharpness slider in Adjust Color) apply an unsharp mask after the colour adjustments.
  Reset Colour clears it.
- The Inspector lists a PDF's title, author, subject and keywords, and whether it is password protected.
- Tools > Show Signatures puts the saved signatures on the right. A click places one on the shown page as one undo step.
  The panels on the right now take turns: Export As, signatures, Adjust Color and the Inspector.

## 0.9.0

- File > Save (Cmd+S) writes the markup, page order and turns into the document and opens it again. A PDF is written
  with its markup as annotations; a picture is written in its own format. File > Revert to Saved opens the file again
  and drops every change since it was saved. The copy the window keeps for a file chosen in the open panel is the one
  Save replaces.
- Edit > Copy (Cmd+C) puts the text of a PDF page on the clipboard, or only the lines inside the selection when there
  is one; a picture goes to the clipboard as PNG. Edit > Select All (Cmd+A) selects the shown page. Copy works by
  line, not by word: the extraction does not give the position of each word.
- Tools > Text Box places a text box at a click and takes the typed text; it is saved into the PDF as a FreeText
  annotation in Helvetica. Text boxes are not burned into exported pictures yet.
- Tools > Star, Polygon and Speech Bubble draw closed shapes. They are saved into the PDF as polygon annotations and
  burn into exports.
- Tools > Redact draws a black box over a picture; the box is burned into saved and exported pictures. It is not offered
  for a PDF, whose text under a box would stay.
- Tools > Save Signature keeps the strokes drawn on the shown page (Sign tool) in a library of up to nine.
  Tools > Insert Signature places one in the middle of the page as drawn strokes, at one undo step.
- View > Hide Toolbar (Cmd+Option+T) hides the toolbar and gives its height to the page.
- Adjust > Show Adjust Color shows sliders for exposure, contrast, saturation and temperature on the right side. A drag
  is one undo step. Sharpness has no filter yet.
- Settings menu: Remember Last Page reopens a document on the page it was last shown on; Open PDFs as Contact Sheet
  opens them as the grid; Annotation Author sets the author written into new markup (typed in the status line).

## 0.8.0

- Tools > Select drags out a rectangle on the page. Tools > Crop (Cmd+K) keeps the selection of a picture and moves its
  markup with it; View > Zoom to Selection fills the window with it.
- Tools > Flip Horizontal and Flip Vertical mirror a picture and its markup. Tools > Instant Alpha clears the region of
  one colour around a click to transparent, and Tools > Remove Background clears the background that reaches the
  picture's edge. All of these can be undone, and Revert to Original brings the pixels back. PNG export now keeps
  transparency; JPEG export lays a transparent picture over white.
- New markup: Oval, Line, Arrow, Underline and Strike Through. They draw in the window, burn into exports and are saved
  into the PDF as Circle, Line, Underline and StrikeOut annotations.
- Edit > Find (Cmd+F) searches the text of a PDF: type the text in the status line and press Return. The matching lines
  are listed in the sidebar (View > Search Results) and Find Next and Find Previous (Cmd+G, Shift+Cmd+G) step through
  the pages that hold them.
- View > Markup and View > Bookmarks list every mark and bookmark in the sidebar; a click goes to its page.
- View > Contact Sheet (Cmd+2) shows every page in a grid. View > Slideshow (Shift+Cmd+F) shows the pages one after
  another without the toolbar and sidebar; arrows, space and Return step, Escape ends it.
- Tools > Show Inspector (Cmd+I) lists the file's name, kind, size, pages or pixels, markup counts and, for a JPEG, its
  camera, exposure and location. Tools > Show Location Info shows the place a picture was taken. Tools > Show Magnifier
  enlarges the picture under the pointer.
- File > New from Clipboard (Cmd+N) opens a picture from the clipboard.
- Tools > Adjust Size (Cmd+Option+I) resamples a picture: type a width, a width x height or a percentage in the status
  line and press Return. File > JPEG Quality chooses High, Medium or Low for exported JPEGs.

## 0.7.0

- File > Open (Cmd+O) shows the open panel for PDFs and pictures. The chosen file is copied into the app's data folder and opened from there.
- File > Open Recent lists the last ten files opened, with Clear Menu. Files that are gone are dropped.

## 0.6.0

- Edit > Undo (Cmd+Z) and Redo (Cmd+Shift+Z) step back and forward through markup, page moves and deletes, turns and
  colour changes.
- File > Revert to Original drops all markup, restores the page order and turns and resets the colour. It can be undone.
- A new Go menu holds Previous, Next, First Page, Last Page, Go to Page (Cmd+Option+G, type the number, Return), Back
  and Forward through the pages visited (Cmd+Option+B, Cmd+Option+F), and bookmarks: Add Bookmark (Cmd+D) toggles one on
  the shown page, Previous and Next Bookmark jump between them. Bookmarks are kept beside the markup.
- View > Hide Sidebar and Show Sidebar (Cmd+Option+S).

## 0.5.1

- File > Print... (Cmd+P) prints the document as it is marked up: a PDF's page order, turns, markup and deletions are
  written to a copy under the app's data folder by the PDF worker, and that copy is sent to the default printer. A
  picture prints its marked up page, with Adjust Color and turns, as a PNG. The original file is not changed.

## 0.5.0

- File > Print... (Cmd+P) sends the open file to the default printer through the shared print operation. It prints the
  file as opened: markup is not drawn into the printout yet.

## 0.4.0

- A PDF opens on its own worker thread: the window stays responsive while the file is read, and pages and sidebar thumbnails are drawn by the worker when they are shown.
- Sidebar switches between Pages and Outline (the PDF outline; a click goes to its page).
- Move Page Up and Move Page Down, and Delete Page, reorder and remove pages of a PDF. Rotate Left and Right keep their turn with the page.
- Sign (Tools menu and toolbar) draws a freehand stroke, for a signature. It is drawn on export too.
- Export PDF (File menu) writes `<name>-markup.pdf`: the original file with its markup as annotations (highlights, rectangles, notes and strokes, each with an appearance), the page order and the turns, appended as an incremental update. The original file is not changed. An encrypted PDF cannot be saved this way.
- Export at 25%, 50%, 100% and 200% (File menu) sets the size of PNG and JPEG exports. A PDF page is drawn at that size.

## 0.3.0

- Opening a PDF reads its page list only. Thumbnails are drawn the first time the sidebar shows them and kept after that, so a long document opens at once.
- Export JPEG (File menu) writes the current page as `<name>-markup.jpg` with rotation, Adjust Color and markup, like Export PNG.

## 0.2.0

- PDF pages are drawn by the PDF engine in `cg::pdf` (paths, text, images, shadings), so they look as they do in the file. Pages are drawn when shown, with thumbnails drawn at open. Text reflow is gone from Viewer.
- PNG export works for PDF pages too, with markup burned in.

## 0.1.0

- New Viewer app (`is.olibuijr.preview.desktop`): opens PNG and JPEG images and PDF files.
- Zoom in, zoom out, actual size and fit to window; rotate left and right; page navigation.
- Thumbnails sidebar for multi-page documents.
- Markup: highlight, text note, rectangle and rounded rectangle. Markup is kept in a sidecar file next to the app's data and drawn on export.
- Adjust Color: exposure, saturation and contrast through the image filter API.
- Export the current page to PNG with its markup.
