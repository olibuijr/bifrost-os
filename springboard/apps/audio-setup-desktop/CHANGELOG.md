# Changelog

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
