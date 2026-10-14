# Changelog

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
