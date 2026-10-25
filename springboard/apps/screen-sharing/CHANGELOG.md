# Changelog

## 0.1.6

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.5

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.4

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## Unreleased

- Password (VNC) authentication: a Password box next to the address, Tab moves between them, and the password is used only when the server asks for one and is forgotten once connected. Servers that need a password and get none say so.

## 0.1.3 — 2026-10-14

- Remove the unused connection polling hook; dispatch completions and the shell service pipe wake the viewer when its state changes.
- Add the store icon asset.

## 0.1.2

- Move the RFB reader and writer onto utility dispatch tasks; connection events wake the app through the main dispatch queue instead of a frame polling timer.

## 0.1.1 — 2026-10-14

- Empty areas around the remote display follow the active appearance; the app enables sudden termination because it has no unsaved document state.
## 0.1.0

- New Screen Sharing app: type the address of a computer that serves VNC (RFB 3.3, 3.7 or 3.8) and Connect. The remote desktop shows scaled to fit or at actual size.
- Pointer and keyboard go to the remote computer; View Only turns input off. Right-click sends a secondary click.
- Clipboard moves both ways: text the server copies is put on this computer's clipboard, and Send Clipboard sends this computer's text.
- Not yet: password (VNC) authentication, encrypted transports, scrolling in actual-size mode, and other encodings such as Tight or ZRLE.
