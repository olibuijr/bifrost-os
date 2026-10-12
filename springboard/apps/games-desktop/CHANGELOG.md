# Changelog

## 1.4.1 — 2026-10-12

- Dark appearance: the main content area fills with the dark content grey instead of pure black, as stock dark content views do; sidebars and cards keep their colours.

## 1.4.0

- Winning a game now shows a short burst of confetti over the board. It plays once for each win: a game that is already won does not repeat it, and the next game's win shows it again. The burst lasts about two seconds and fades out.
- The confetti uses a small 3D particle model, so pieces spread a little toward the viewer and shrink as they move away. Nothing else on the board moves.
- The Games details dump reports how many confetti dots are on screen, so a test can check the burst.
- Sound for the win is not added yet: the audio service plays queued files at one volume and cannot place a sound left or right.

## 1.3.0

- Games use the shared random source from the core crate for every shuffle and spawn. The numbers are the same as before, so a seeded deal and the 2048 tile placement do not change.
- Each game's won or lost state is now a small state machine with explicit rules: a game in progress can be won or lost, a won game can still end, and a finished game only starts over with a new game. A stray end or move on a finished board is refused.
- Game controllers work with 2048: the direction pad or the left stick slides the tiles, and the South button starts a new game once the current one is over (a game in progress is never dropped by a press).
- A controller plugged in or out shows a line in the message row ("Controller connected: <name>"); the line goes when you next touch the board.
- Low Power Mode, or a device running hot, lowers the motion: the Minesweeper clock redraws every 5 seconds instead of every second. Best times are still exact.
- The app knows the main display's visible size from the system, and reports it in its state dump. It does not change the layout yet.
- Minesweeper and Solitaire have no controller cursor yet; a controller only starts a new game in them, once the game is over.

## 1.2.3

- App compliance re-check: the source list and the unified toolbar are drawn on the first screen at 1200x780 (Light and Dark), so their rows and buttons are published as controls; no visual or behaviour change.

## 1.2.2

- Playing cards follow Light and Dark: the face and back use the card background and the black suits the label colour; red suits are unchanged.
- The 2048 and Minesweeper boards are placed from named row tokens instead of fixed points, so they keep their place as the window size changes.
- The app icon is drawn from the system's named colours.
- Opts in to sudden termination: nothing is unsaved (best scores are saved as they are set).

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Golden Gate look: a full-size window with a source-list sidebar (2048, Minesweeper, Solitaire) and a unified toolbar
  holding the game's title and commands (New Game, Flag, Undo, Draw 1/3, Auto); the tab row and bottom buttons are gone.
- Follows Light and Dark: backgrounds, boards, labels and the 2048 tiles use the system's named colours.
- Toolbar buttons are published as controls, so they work under the window-move band and with assistive technology.
- Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 8 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- Games for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
