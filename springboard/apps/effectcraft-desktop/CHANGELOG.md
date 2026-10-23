## 0.7.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.15.0

- Upstream effectcraft v0.15.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * security: enforce HTTPS on curl downloads (SonarCloud S6506) by @dylanmtaylor in https://github.com/storytold/effectcraft/pull/2059
  * README: the app icon above the title by @bflatastic in https://github.com/storytold/effectcraft/pull/2099

## 0.14.0

- Upstream effectcraft v0.14.0.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/effectcraft/pull/1803
  * docs: drop references to competing commercial software by @dylanmtaylor in https://github.com/storytold/effectcraft/pull/1773
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/effectcraft/pull/1962
  **Full Changelog**: https://github.com/storytold/effectcraft/compare/v0.13.0...v0.14.0

## 0.11.2

- Upstream effectcraft v0.11.1.
  ## What's Changed
  * Atomic writes: a unique, exclusively created temp file per write (#1233) by @DWAA1660 in https://github.com/storytold/effectcraft/pull/1345
  * Offloaded analyses: merge the result instead of replacing the group (#1249) by @DWAA1660 in https://github.com/storytold/effectcraft/pull/1366
  * EPS: idiv and mod of -2^63 by -1 no longer panic by @dotCooCoo in https://github.com/storytold/effectcraft/pull/1223
  * Prevent duplicate automation edits after lost replies by @onlymoein in https://github.com/storytold/effectcraft/pull/1333

## 0.11.1

- Upstream effectcraft v0.11.1.
  ## What's Changed
  * Atomic writes: a unique, exclusively created temp file per write (#1233) by @DWAA1660 in https://github.com/storytold/effectcraft/pull/1345
  * Offloaded analyses: merge the result instead of replacing the group (#1249) by @DWAA1660 in https://github.com/storytold/effectcraft/pull/1366
  * EPS: idiv and mod of -2^63 by -1 no longer panic by @dotCooCoo in https://github.com/storytold/effectcraft/pull/1223
  * Prevent duplicate automation edits after lost replies by @onlymoein in https://github.com/storytold/effectcraft/pull/1333

## 0.10.1

- Upstream effectcraft v0.10.0.
  ## What's Changed
  * Track matte on an adjustment layer limits where its effects show by @marcoarnulfo in https://github.com/storytold/effectcraft/pull/1204
  * UI: Quick Apply keeps an IME composition while pinyin is typed by @cjcwwww in https://github.com/storytold/effectcraft/pull/1206
  * fix(effects): CC Particle World / Systems II replace their layer; Time effects see earlier effects by @eliasx45 in https://github.com/storytold/effectcraft/pull/1111
  * Tracker: a track point can be grabbed when zoomed out by @marcoarnulfo in https://github.com/storytold/effectcraft/pull/1112

## 0.10.0

- Upstream effectcraft v0.10.0.
  ## What's Changed
  * Track matte on an adjustment layer limits where its effects show by @marcoarnulfo in https://github.com/storytold/effectcraft/pull/1204
  * UI: Quick Apply keeps an IME composition while pinyin is typed by @cjcwwww in https://github.com/storytold/effectcraft/pull/1206
  * fix(effects): CC Particle World / Systems II replace their layer; Time effects see earlier effects by @eliasx45 in https://github.com/storytold/effectcraft/pull/1111
  * Tracker: a track point can be grabbed when zoomed out by @marcoarnulfo in https://github.com/storytold/effectcraft/pull/1112

## 0.9.3

- Upstream effectcraft v0.9.2.
  ## What's Changed
  * Share docking transactions and preserve viewer lifecycle by @skylartaylor in https://github.com/storytold/effectcraft/pull/689
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/effectcraft/pull/801
  * #678: dropped Photoshop files show the Import Photoshop File dialog by @vangel76 in https://github.com/storytold/effectcraft/pull/734
  * #729: import STL 3D models (binary and ASCII) by @vangel76 in https://github.com/storytold/effectcraft/pull/735

## 0.9.2

- Upstream effectcraft v0.9.2.
  ## What's Changed
  * Share docking transactions and preserve viewer lifecycle by @skylartaylor in https://github.com/storytold/effectcraft/pull/689
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/effectcraft/pull/801
  * #678: dropped Photoshop files show the Import Photoshop File dialog by @vangel76 in https://github.com/storytold/effectcraft/pull/734
  * #729: import STL 3D models (binary and ASCII) by @vangel76 in https://github.com/storytold/effectcraft/pull/735

## 0.8.0

- Upstream effectcraft v0.8.0.
  ## What's Changed
  * Land 49 community PRs, with follow-ups by @bflatastic in https://github.com/storytold/effectcraft/pull/598
  * i18n: add Brazilian Portuguese (pt-br) UI translation by @RicSchonfelder in https://github.com/storytold/effectcraft/pull/217
  * G2: Fix effect self-inputs, CC Composite, and control activation by @cdinhoo in https://github.com/storytold/effectcraft/pull/423
  * Add rendered-frame fidelity scoring by @vbasky in https://github.com/storytold/effectcraft/pull/427

## 0.7.0

- Initial BifrOSt App Store package of upstream effectcraft v0.7.0.
