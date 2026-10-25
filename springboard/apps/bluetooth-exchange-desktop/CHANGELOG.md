# Changelog

## 0.1.7

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.6

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.5

- Fix: the toolbar starts after the traffic lights instead of under them (the SDK toolbar now keeps clear of the window controls).

## 0.1.4 — 2026-10-14

- Run transfers on the serial utility dispatch queue, wake on replies, and cancel queued/active work when the window closes.

## 0.1.3 — 2026-10-14

- Derive the initial content geometry from the host-provided window size instead of assuming a default resolution.

## 0.1.2 — 2026-10-14

- Add the generated app compliance harness.

## 0.1.1 — 2026-10-14

- Add the Bluetooth File Exchange store icon.

## 0.1.0

- First version. A device list from the shell's Bluetooth service (paired devices first, Scan for nearby ones, Pair for
  an unpaired one). Files in Downloads are queued to the selected device and sent one at a time over OBEX object push
  through BlueZ's obexd on the session bus, with progress, cancel and a clear-finished action. Files in
  Downloads/Bluetooth are listed as received.
