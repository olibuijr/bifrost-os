# Changelog

## 0.2.0

- Menus: File (Take Photo, Export, Delete), View (Photo, Four-Up, Video Clip, countdown, Auto Flip, Screen Flash, Play
  Slideshow) and Camera (every camera the system lists, the one in use checked).
- Distortion effects Bulge, Twirl and Mirror; Auto Flip mirrors the viewfinder and every capture; Screen Flash can be
  switched off; Delete (key or menu) removes the selected capture's file; a full-window slideshow of the captures.
- Still missing: backdrops (needs background segmentation), full screen (no app-level window API yet).

## 0.1.0

- New app Photo Booth (`is.olibuijr.photobooth.desktop`): live viewfinder from the system camera, Photo, 4-up Burst
  (four stills and one contact sheet) and a short Clip (Motion JPEG AVI), a Off, 3 s and 10 s countdown with a flash, an
  effect row (Mono, Noir, Sepia, Warm, Cool, Vivid, Posterize, Vignette, Invert) built on the Core Image style filters,
  a strip of captures, and Export through the save panel. Captures go to `~/Pictures/Photo Booth Library`.
- Still missing: share sheet (needs a document grant per file), camera picker when several cameras are attached,
  mirror and aspect controls, video encoding in a modern codec (the clip is Motion JPEG), and background encoding
  (the shutter encodes on the UI thread).
