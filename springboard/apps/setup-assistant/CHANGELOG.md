# Changelog

## 0.1.6

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.5

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.2

- Exported the existing vector icon as the required 180 × 180 PNG.

## 0.1.1 — 2026-10-14

- Step titles use the shared large-title widget, layout initializes from the configured window size, and the switch knob uses a scheme-aware surface colour.

## 0.1.0 — 2026-10-13

- First version: the first-run window after install. It walks the time zone, keyboard, network, accessibility shortcut,
  account, appearance, privacy (Location Services and Analytics & Diagnostics, both off by default) and BifrOSt Account
  sign-in (Sign In opens the account page in the browser), then writes the state file the shell reads at start-up.
