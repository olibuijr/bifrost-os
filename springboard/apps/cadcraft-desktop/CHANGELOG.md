## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.17.1

- Upstream cadcraft v0.17.1.
  **Full Changelog**: https://github.com/storytold/cadcraft/compare/v0.17.0...v0.17.1

## 0.17.0

- Upstream cadcraft v0.17.0.
  ## What's Changed
  * -ATTDEF at the command line: attribute modes, AFLAGS, and the text's prompts by @2ez4gcx in https://github.com/storytold/cadcraft/pull/2597
  * -WBLOCK at the command line, as AutoCAD runs it by @2ez4gcx in https://github.com/storytold/cadcraft/pull/2594
  * LEADER at the command line, as AutoCAD runs it by @2ez4gcx in https://github.com/storytold/cadcraft/pull/2596
  * Thai text: stacked marks, Thai fonts first, MTEXT word wrap by @awillfox in https://github.com/storytold/cadcraft/pull/2591

## 0.16.0

- Upstream cadcraft v0.16.0.
  ## What's Changed
  * Validate new linetype names in the rename command by @Silvertree2010 in https://github.com/storytold/cadcraft/pull/2519
  * Updates: shared check-result dialog from craft-update by @aiongg in https://github.com/storytold/cadcraft/pull/2530
  * docs: tidy agent, contributor and asset docs by @aiongg in https://github.com/storytold/cadcraft/pull/2527
  * Backups of drawings sharing a name no longer overwrite each other by @tanish2k09 in https://github.com/storytold/cadcraft/pull/2537

## 0.15.1

- Upstream cadcraft v0.15.1.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/2413
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/2425
  * PEDIT Multiple at the command line by @aldozar in https://github.com/storytold/cadcraft/pull/2225
  * Reject out-of-range LUNITS, LUPREC, AUNITS and AUPREC in SETVAR by @Silvertree2010 in https://github.com/storytold/cadcraft/pull/2269

## 0.14.0

- Upstream cadcraft v0.14.0.
  ## What's Changed
  * Migrate CADCraft to egui 0.37 by @skylartaylor in https://github.com/storytold/cadcraft/pull/2119
  * craft-pdf and its PDF backends back at craft-libs 6e7f285 by @aiongg in https://github.com/storytold/cadcraft/pull/2147
  * macOS menu choices and their shortcuts run at once by @aiongg in https://github.com/storytold/cadcraft/pull/2140
  * "Save changes?" and Drawing Recovery appear at once by @aiongg in https://github.com/storytold/cadcraft/pull/2144

## 0.13.0

- Upstream cadcraft v0.13.0.
  ## What's Changed
  * docs: document security limits and regression guarantees by @skylartaylor in https://github.com/storytold/cadcraft/pull/1963
  * chore(deps): bump the cargo-dependencies group across 1 directory with 17 updates by @dependabot[bot] in https://github.com/storytold/cadcraft/pull/1957
  * web: allow browser zoom in the viewport meta (SonarCloud Web:S7926) by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1974
  * packaging: use [[ ]] tests, named locals and explicit returns in shell scripts (SonarCloud S7688/S7679/S7682) by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1972

## 0.12.0

- Upstream cadcraft v0.12.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * security: write files atomically through an exclusive temp file (CWE-59) by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1840
  * security: do not follow directory symlinks in cargo xtask stats (CWE-400) by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1845

## 0.11.0

- Upstream cadcraft v0.11.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/cadcraft/pull/1566
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/cadcraft/pull/1727

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
