# Changelog

## 0.1.13

- The layout uses points instead of pixels on scaled displays.

## 0.1.12

- One add control instead of two.

## 0.1.11

- Rebuilt with the 0.10.40 SDK: a background app redraws at its new size after a resize.

## 0.1.10

- Rebuilt with the current SDK: the app redraws after an action from the shell (Look Up, Services, Spotlight) and accepts the shell's account-credential connections.

## 0.1.9

- Rebuilt with the current SDK and renderer: shell actions such as Look Up and Services reach the app, and vibrant text draws inside clipped areas.

## 0.1.8

- Rebuilt with the current renderer and SDK: Chinese, Japanese and Korean text draws whole, and a file opened with this app from Files or the Dock opens as a document.

## 0.1.7

- Fix: the toolbar starts after the traffic lights instead of under them (the SDK toolbar now keeps clear of the window controls).

## 0.1.6 — 2026-10-14

- Include the 180 x 180 waveform app icon in the source package.

## 0.1.5

- Added the SDK and design compliance test harness.

## 0.1.4

- Added a 180 x 180 PNG app icon generated from the existing waveform artwork and removed the unused storage permission.

## 0.1.3 — 2026-10-12

- Added the app icon (a waveform tile) so the app can be published.

## 0.1.2 — 2026-10-12

- Rebuilt on the BifrOSt 0.6.4 SDK: the shared shell/app wire codec, frame and list caps, and the window materials API.

## 0.1.1

- The sidebar section headers come from the device kind (`Kind::section`), and the unused draft constructor is removed.

## 0.1.0

- New app. The sidebar lists the audio devices (hardware, then aggregates and multi-output devices). The detail view
  shows the output format, sample rate, channels, buffer size, latency, safety offset and volume of the chosen device.
- Aggregate and multi-output devices are created and edited in the editor: a name, the kind, the member devices, and
  the clock source. The server validates the request; a refusal is shown under the form.
- Remove deletes the selected aggregate or multi-output device. The list refreshes every two seconds.
