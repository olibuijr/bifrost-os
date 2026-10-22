## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.10.0

- Upstream cadcraft v0.10.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: drop references to competing commercial software by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1545
  * ci: configure Dependabot for Cargo and GitHub Actions by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1577

## 0.8.1

- Upstream cadcraft v0.8.0.
  ## What's Changed
  * RECTANG: the other corner ignores ortho, so the rectangle keeps its area by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/1117
  * ci: add shared FreeBSD, Windows ARM64, and nix checks by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1382
  * Bound control-port input, and follow stored paths only on this computer or the drawing's share by @skylartaylor in https://github.com/storytold/cadcraft/pull/1518
  **Full Changelog**: https://github.com/storytold/cadcraft/compare/v0.7.0...v0.8.0

## 0.8.0

- Upstream cadcraft v0.8.0.
  ## What's Changed
  * RECTANG: the other corner ignores ortho, so the rectangle keeps its area by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/1117
  * ci: add shared FreeBSD, Windows ARM64, and nix checks by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1382
  * Bound control-port input, and follow stored paths only on this computer or the drawing's share by @skylartaylor in https://github.com/storytold/cadcraft/pull/1518
  **Full Changelog**: https://github.com/storytold/cadcraft/compare/v0.7.0...v0.8.0

## 0.7.1

- Upstream cadcraft v0.7.0.
  ## What's Changed
  * Select: pick a SOLID inside its fill, not its bounding rectangle by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/747
  * View: refuse a view that maps to an infinite viewport view, keeping the viewport by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/749
  * Plotting: honor model-space center and X/Y offsets by @cyfung1031 in https://github.com/storytold/cadcraft/pull/751
  * Hatch and Gradient dialog (create and edit) by @LloydNicholson in https://github.com/storytold/cadcraft/pull/753

## 0.7.0

- Upstream cadcraft v0.7.0.
  ## What's Changed
  * Select: pick a SOLID inside its fill, not its bounding rectangle by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/747
  * View: refuse a view that maps to an infinite viewport view, keeping the viewport by @marcoarnulfo in https://github.com/storytold/cadcraft/pull/749
  * Plotting: honor model-space center and X/Y offsets by @cyfung1031 in https://github.com/storytold/cadcraft/pull/751
  * Hatch and Gradient dialog (create and edit) by @LloydNicholson in https://github.com/storytold/cadcraft/pull/753

## 0.6.3

- Upstream cadcraft v0.6.2.
  ## What's Changed
  * AUDIT and RECOVER: find and fix database errors, open damaged drawings repaired (part of #406) by @vangel76 in https://github.com/storytold/cadcraft/pull/411
  * Plot styles: colour-dependent tables (monochrome, grayscale) apply to PDF plots and layouts (part of #410) by @vangel76 in https://github.com/storytold/cadcraft/pull/412
  * MTEXT: Height, Justify, Line spacing, Rotation, Style, Width, Columns (part of #408) by @vangel76 in https://github.com/storytold/cadcraft/pull/415
  * SPLINE: Knots, start/end Tangency, toLerance and Object (part of #408) by @vangel76 in https://github.com/storytold/cadcraft/pull/413

## 0.6.2

- Upstream cadcraft v0.6.2.
  ## What's Changed
  * AUDIT and RECOVER: find and fix database errors, open damaged drawings repaired (part of #406) by @vangel76 in https://github.com/storytold/cadcraft/pull/411
  * Plot styles: colour-dependent tables (monochrome, grayscale) apply to PDF plots and layouts (part of #410) by @vangel76 in https://github.com/storytold/cadcraft/pull/412
  * MTEXT: Height, Justify, Line spacing, Rotation, Style, Width, Columns (part of #408) by @vangel76 in https://github.com/storytold/cadcraft/pull/415
  * SPLINE: Knots, start/end Tangency, toLerance and Object (part of #408) by @vangel76 in https://github.com/storytold/cadcraft/pull/413

## 0.5.1

- Upstream cadcraft v0.5.0.
  ## What's Changed
  * Render tests: drop the always-true assert in degenerate_input by @kaffeochknivar in https://github.com/storytold/cadcraft/pull/37
  * Keep elevations when EXPLODE splits a 3D polyline (#88) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/138
  * Reject out-of-domain values in PARAMETERS instead of dropping the constraint (#93) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/139
  * Apply DIMOVERRIDE text together with dimension variables (#108) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/141

## 0.5.0

- Upstream cadcraft v0.5.0.
  ## What's Changed
  * Render tests: drop the always-true assert in degenerate_input by @kaffeochknivar in https://github.com/storytold/cadcraft/pull/37
  * Keep elevations when EXPLODE splits a 3D polyline (#88) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/138
  * Reject out-of-domain values in PARAMETERS instead of dropping the constraint (#93) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/139
  * Apply DIMOVERRIDE text together with dimension variables (#108) by @MarekSpodBiedry in https://github.com/storytold/cadcraft/pull/141

## 0.4.0

- Initial BifrOSt App Store package of upstream cadcraft v0.4.0.
