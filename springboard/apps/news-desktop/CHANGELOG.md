# Changelog

## 1.2.2 — 2026-10-11

- Compliance: feeds and pictures load on the shared dispatch pool (`bifrost_app::dispatch`) instead of a worker pool with a poll loop; the loop sleeps until the next feed is due and a finished job wakes it through the main queue.
- The app enables sudden termination (it keeps no unsaved state), so the shell can quit it at once.
- The source list rows (Today and each feed) are published as accessibility controls, and each can be activated.
- Corner radii come from the Settings card and plate tokens; the icon colours are named artwork constants.

## 1.2.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.2.0 — 2026-10-10

- Looks like News on the Mac: a full-size window with a source list (Today, then each feed under Sources) in place of the tab strip, and a unified toolbar with Back and Refresh in place of the iPad navigation bar.
- Follows Light and Dark: every colour is a system Pair token; the toolbar buttons are published as controls.
- Needs system 0.4.84.

## 1.1.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.1.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 3 padding and gap values moved to the nearest standard step (at most 2 pt).
- Built for system 0.4.62.

## 1.1.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.1.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.0.0 — 2026-10-08

- News for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
