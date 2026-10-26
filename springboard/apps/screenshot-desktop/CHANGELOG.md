# Changelog

## 0.1.6

- The Screenshot window ships with the app, so it installs and opens.

## 0.1.5

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.4

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.3

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.2

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.1 — 2026-10-14

- Remove the unused storage permission from the options-only model crate; capture and file-writing code is not present yet.
- Add the store icon asset.

## 0.1.0

- Capture options model: target (screen, window, selection), still or recording, destination (Desktop, Documents, Clipboard), timer (off, 5 s, 10 s), pointer and floating thumbnail.
- File names `Screenshot <date> at <time>.png` and `Screen Recording <date> at <time>.mov`.
