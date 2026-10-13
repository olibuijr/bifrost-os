# Changelog

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
