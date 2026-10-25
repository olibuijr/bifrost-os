# Changelog

## 0.1.8

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.7

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.6

- Wireless Diagnostics stops reading the radio while its window is hidden or minimised and starts again when it is shown, repaints only when a reading changes what is on screen, and kills an `iw` that does not answer in time (5 s for a link read, 30 s for a scan).

## 0.1.5

- Fix: under the compliance harness (pinned clock) the app shows a fixed Wi-Fi link and signal history instead of polling the host radio, so its snapshot no longer varies from run to run.

## 0.1.4

- Fix: the toolbar starts after the traffic lights instead of under them (the SDK toolbar now keeps clear of the window controls).

## 0.1.3

- BifrOSt 0.10.0 round (waves 14 and 15): see the system CHANGELOG for this app's changes.

## 0.1.2

- Exported the existing icon at the required 180 × 180 PNG size.

## 0.1.1 — 2026-10-14

- Radio readings and scan results return through the main dispatch queue; idle polling threads and periodic window wakes are gone.

## 0.1.0

- New: Wireless Diagnostics. The Connection card shows the network, BSSID, channel, band, signal, noise, signal-to-noise ratio, a quality rating, and the receive and transmit rates, read from the kernel (`/proc/net/wireless`) and `iw` every second.
- New: a live signal chart of the last two minutes, with a grid at the usual signal levels.
- New: Scan runs a Wi-Fi scan in the background (falls back to the cached results when a fresh scan is refused) and lists nearby networks by signal with channel, band and security.
- New: Save Report writes a plain-text diagnostics report to the app's data folder, and Copy Summary puts the same text on the clipboard.
- New: the pure parsing, quality rating, history and report code is tested.
