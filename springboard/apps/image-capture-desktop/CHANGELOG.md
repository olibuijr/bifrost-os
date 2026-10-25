# Changelog

## 0.1.4

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.3

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.2

- Fix: the toolbar starts after the traffic lights instead of under them (the SDK toolbar now keeps clear of the window controls).

## 0.1.1 — 2026-10-14

- Move disk and scanner requests to a serial utility dispatch queue, with replies waking the app on the main queue; remove in-flight polling, enable sudden termination, and remove the fixed initial window size.
- Add the store icon and derive the initial layout from the host-provided window size.

## 0.1.0

- New: Image Capture. The sidebar lists the cameras and SD cards with a DCIM folder and the scanners `scanimage -L` reports. A camera shows its pictures; pick some (or all) and Import copies them into Pictures, keeping their names. A scanner shows its resolution, colour and format; Preview scans a quick look, and Scan saves the page into Downloads (the scan settings name the folder). The Print & Scan pane lists the scanners too.
