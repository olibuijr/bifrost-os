# Changelog

## 0.1.11

- The save sheet takes Cmd+D for Don't Save, and Tab shows a focus ring that Space presses.

## 0.1.10

- The red close button closes the active script and asks before closing an edited one.

## 0.1.9

- Close (Cmd+W) asks to save an edited script; an unsaved script autosaved at Quit opens again at the next launch.

## 0.1.8

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.7

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.6

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.5

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.1.3 — 2026-10-14

- Use the shared small-control corner radius token.

## 0.1.2

- Added the SDK and design compliance test harness.

## 0.1.1

- Exported the app icon at the required 180 x 180 pixel size.

## 0.1.0

- First version: a shell script editor with Run and Stop, a result pane for standard output and standard error, the status of the last run, and the usual File menu (New, Open, Open Recent, Save, Save As) through the shell's document panels.
- Scripts run in /bin/sh from the home directory on a worker thread, so the window keeps drawing while they work.
