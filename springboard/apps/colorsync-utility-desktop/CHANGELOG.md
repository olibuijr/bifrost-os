# Changelog

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

- Draw and hit-test profile rows with the Golden Gate source-list widget.

## 0.1.2

- Added the SDK and design compliance test harness.

## 0.1.1

- Exported the app icon at the required 180 x 180 pixel size.

## 0.1.0

- New app: Colour Utility lists the ICC colour profiles in the system folders (`/usr/share/color/icc`,
  `/usr/local/share/color/icc`) and in the user's folders (`~/.local/share/color/icc`, `~/.color/icc`).
- Header: the profile size, version, device class, colour space, connection space, creation date, platform,
  manufacturer, model, creator, rendering intent and PCS illuminant.
- Tags: every entry of the tag table with its name, data type, offset and size. A tag that lies outside the file is
  marked.
- Verify: structural checks on the header, the tag table, the tag bounds and alignment, duplicate tags, required tags
  of matrix and greyscale profiles, and the data types of colorant and curve tags.
- Gamut: the red, green and blue primaries and the white point of a matrix RGB profile, drawn on the CIE 1931 xy
  chart with the spectral locus, and the area against sRGB. LUT-based profiles report that they carry no colorants.
