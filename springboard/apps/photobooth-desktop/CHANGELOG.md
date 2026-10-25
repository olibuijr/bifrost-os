# Changelog

## 0.5.7

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.5.6

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.5.5

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.5.4 — 2026-10-14

- Require shell 0.8.0 for the launcher's camera permission.
- Keep mode, timer, shutter and export hit targets inside the controls bar without overlap as the window narrows.
- Document the intentional white camera flash and shutter face fills.

## 0.5.3 — 2026-10-14

- Named preview and clip-frame pixel limits by their media purpose rather than as window dimensions.

## 0.5.2

- The app is a full-size window; preview, capture-strip and selected-capture colors follow the appearance, and the existing icon is exported at the required 180 × 180 PNG size.

## 0.5.1 — 2026-10-14

- Capture editing commands are in the Photo menu so the app does not duplicate the shell Edit menu.

## 0.5.0

- Choosing a camera in the Camera menu reads the camera list again first. A camera unplugged since the menu opened is
  reported in the status line instead of opened, and the choice is matched by its device node, not its position.
- Edit > Auto Flip has a key equivalent, cmd+shift+m.
- View > Full Screen Viewfinder moved from cmd+ctrl+f to cmd+shift+f: the shell takes cmd+ctrl+f for window full screen
  before the app's menus see it.
- Still missing: an in-app Trash view, photo and movie backdrops, full screen for the window itself (no app-level window
  API), a Window or Help menu, slideshow transitions other than the fade, and lab checks of Screen Flash, Backdrops,
  Full Screen Viewfinder and the Camera menu on Midget.

## 0.4.0

- Delete moves the capture into the desktop Trash folder (`$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`) with the
  trash record of where it came from and when it was deleted, so the file manager can show it and put it back.
  Undo Delete (cmd+z) moves it out of the Trash to its place in the strip; if a picture has taken its name since, the
  restored one gets a free name.
- The screen flash timing moved into the model, where it is tested headless.
- Still missing: an in-app Trash view, photo and movie backdrops, full screen for the window itself (no app-level
  window API), and lab checks of Screen Flash, Backdrops, Full Screen Viewfinder and the Camera menu on Midget.

## 0.2.0

- Menus: File (Take Photo, Export, Delete), View (Photo, Four-Up, Video Clip, countdown, Auto Flip, Screen Flash, Play
  Slideshow) and Camera (every camera the system lists, the one in use checked).
- Distortion effects Bulge, Twirl and Mirror; Auto Flip mirrors the viewfinder and every capture; Screen Flash can be
  switched off; Delete (key or menu) removes the selected capture's file; a full-window slideshow of the captures.
- Still missing: backdrops (needs background segmentation), full screen (no app-level window API yet).

## 0.2.0

- Menus in the menu bar. File: Take Photo, Export and Show Photo Booth Library. Edit: Delete and Auto Flip. View:
  Slideshow, Full Screen Viewfinder, Screen Flash and Backdrop. Camera: every camera the system lists.
- Delete removes the selected capture from the strip and its file from the library, from the Delete key or the menu;
  the capture that takes its place is selected. The file is removed at once; there is no Trash or undo yet.
- Auto Flip turns the picture left to right in the viewfinder and in every capture, burst shot and clip frame. It is on
  by default and remembered.
- Screen Flash can be switched off. The setting is remembered.
- Camera menu: choose between several cameras. The choice is remembered and the list is read again whenever the app
  comes back to the front.
- Slideshow: the captures of the session one after the other over the whole window. Arrow keys step, Space pauses,
  Escape or a click ends it.
- Full Screen Viewfinder hides the effects row, the strip and the controls so the picture fills the window; Escape
  brings them back and Return takes the picture. The window itself still goes full screen with the shell's shortcut.
- Three distortion effects in the effects row: Bulge, Twirl and Mirror.
- Backdrops (Sunset, Ocean, Forest, Night Sky): after the choice the person steps out of the frame, the app takes a
  picture of the empty scene three seconds later, and from then on everything that differs from that picture stays
  while the rest is replaced by the gradient. It is background subtraction, not a person-detection model, so a moved
  camera or a changed light needs View > Backdrop > Retake Empty Scene.

## 0.3.0

- Edit > Undo Delete (cmd+z) puts back the last capture Delete removed, for the rest of the session. There is still no
  Trash: the file is removed at once.
- Key equivalents for the daily commands: Show Photo Booth Library (cmd+shift+l), Slideshow (cmd+shift+p) and Full
  Screen Viewfinder (cmd+ctrl+f). Edit > Auto Flip has none.
- View > Distortion Strength (Subtle, Normal, Strong) sets how far Bulge and Twirl pull; the choice is remembered.
- The slideshow fades each capture into the next over 0.4 s instead of cutting.
- The Camera menu picks up a camera plugged in after launch: while no camera is open the list is read every 2 s.
- Still missing: Trash, full screen for the window itself (no app-level window API), photo and movie backdrops, and a
  lab check of Screen Flash, Backdrops and Full Screen Viewfinder on Midget.

## 0.2.0

- Menus: File (Take Photo, Export, Delete), View (Photo, Four-Up, Video Clip, countdown, Auto Flip, Screen Flash, Play
  Slideshow) and Camera (every camera the system lists, the one in use checked).
- Distortion effects Bulge, Twirl and Mirror; Auto Flip mirrors the viewfinder and every capture; Screen Flash can be
  switched off; Delete (key or menu) removes the selected capture's file; a full-window slideshow of the captures.
- Still missing: backdrops (needs background segmentation), full screen (no app-level window API yet).

## 0.2.0

- Menus in the menu bar. File: Take Photo, Export and Show Photo Booth Library. Edit: Delete and Auto Flip. View:
  Slideshow, Full Screen Viewfinder, Screen Flash and Backdrop. Camera: every camera the system lists.
- Delete removes the selected capture from the strip and its file from the library, from the Delete key or the menu;
  the capture that takes its place is selected. The file is removed at once; there is no Trash or undo yet.
- Auto Flip turns the picture left to right in the viewfinder and in every capture, burst shot and clip frame. It is on
  by default and remembered.
- Screen Flash can be switched off. The setting is remembered.
- Camera menu: choose between several cameras. The choice is remembered and the list is read again whenever the app
  comes back to the front.
- Slideshow: the captures of the session one after the other over the whole window. Arrow keys step, Space pauses,
  Escape or a click ends it.
- Full Screen Viewfinder hides the effects row, the strip and the controls so the picture fills the window; Escape
  brings them back and Return takes the picture. The window itself still goes full screen with the shell's shortcut.
- Three distortion effects in the effects row: Bulge, Twirl and Mirror.
- Backdrops (Sunset, Ocean, Forest, Night Sky): after the choice the person steps out of the frame, the app takes a
  picture of the empty scene three seconds later, and from then on everything that differs from that picture stays
  while the rest is replaced by the gradient. It is background subtraction, not a person-detection model, so a moved
  camera or a changed light needs View > Backdrop > Retake Empty Scene.

## 0.1.0

- New app Photo Booth (`is.olibuijr.photobooth.desktop`): live viewfinder from the system camera, Photo, 4-up Burst
  (four stills and one contact sheet) and a short Clip (Motion JPEG AVI), a Off, 3 s and 10 s countdown with a flash, an
  effect row (Mono, Noir, Sepia, Warm, Cool, Vivid, Posterize, Vignette, Invert) built on the Core Image style filters,
  a strip of captures, and Export through the save panel. Captures go to `~/Pictures/Photo Booth Library`.
- Still missing: share sheet (needs a document grant per file), camera picker when several cameras are attached,
  mirror and aspect controls, video encoding in a modern codec (the clip is Motion JPEG), and background encoding
  (the shutter encodes on the UI thread).
