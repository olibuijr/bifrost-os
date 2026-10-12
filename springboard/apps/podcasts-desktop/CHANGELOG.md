# Changelog

## 1.4.0

- Skips stop at the ends of an episode: a back skip never goes before the start, and a forward skip stops at the end
  once the length is known. The skip buttons use the shared skip-interval rules.
- The speed button steps through 0.5x to 2x and wraps back to 0.5x after 2x. The chosen speed is shown; the engine
  still plays at normal speed until the audio service takes a rate.
- Keyboard transport: Space plays or pauses, and the Left and Right arrows skip back and forward 30 seconds, when no
  text field has focus.

## 1.3.3 — 2026-10-11

- Compliance: feed and download results reach the app loop through a shared inbox filled on the main queue, instead of
  an mpsc channel the loop polled. The Now Playing artwork is sized from the window height, the empty-state text fills
  the list area, and the feed URL field is a capsule. The app icon uses the named system purple and white.

## 1.3.2 — 2026-10-10

- Snow Leopard: feeds, episode downloads and the playback clock no longer use raw threads or polling for results.
  Work runs on the dispatch pool (Utility) and reaches the app loop through the main queue, so the loop wakes when a
  result arrives. The app enables sudden termination: its subscriptions and resume positions are written as they change.

## 1.3.1

- Follows a Light/Dark switch immediately, including while the window is in the background.

## 1.3.0 — 2026-10-10

- Golden Gate layout: a full-size window with a source-list sidebar (Shows and one row per subscription), a unified
  toolbar (title, Add Podcast, Remove Show) and a floating mini player; the iPad back button, large title and bottom
  bar are gone. Light and Dark both follow the system appearance. Needs BifrOSt 0.4.84.

## 1.2.3 — 2026-10-10

- Opens again on BifrOSt 0.4.90 and later: rebuilt against the current SDK, whose app socket is `BIFROST_SOCK`.

## 1.2.2 — 2026-10-09

- Spacing follows the system 4-pt grid: 9 padding and gap values moved to the nearest standard step (at most 2 pt).
- 1 colour now uses the system colour token (same values).
- Built for system 0.4.62.

## 1.2.1 — 2026-10-09

- Text that is shortened to fit now measures at the size it is drawn at, so labels are no longer cut off early.
- Follows the system accessibility settings (Increase Contrast, Reduce Transparency, Show Borders, Bold Text) on system 0.4.61.

## 1.2.0 — 2026-10-09

- Text sizes and spacing come from the shared macOS-style scale, so this app matches the others.
- No gap above the content for a status bar the desktop does not have.

## 1.1.0 — 2026-10-08

- The show list has a large "Podcasts" title under the bar, as Books does. Episodes keep their inline title with Back.

## 1.0.0 — 2026-10-08

- Podcasts for the BifrOSt desktop: the tablet app's code, hosted in a window with keyboard and pointer input.

- Use the desktop package identity for the SDK handshake and supported PC permissions.
- Connect to the Linux audio service path.
- Keep text-field focus without drawing an on-screen keyboard in PC app processes.
- Route physical text, Backspace and Return keys to existing field editors.
