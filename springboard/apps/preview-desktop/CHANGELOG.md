# Changelog

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
