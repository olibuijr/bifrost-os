## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.10.0

- Upstream deckcraft v0.10.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/deckcraft/pull/984
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/deckcraft/pull/1068

## 0.9.0

- Upstream deckcraft v0.9.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Keep bare '&' text when pasting HTML instead of dropping it by @Silvertree2010 in https://github.com/storytold/deckcraft/pull/795
  * Keep multi-paragraph slide titles on one line in outline export by @Silvertree2010 in https://github.com/storytold/deckcraft/pull/796

## 0.8.0

- Upstream deckcraft v0.8.0.
  ## What's Changed
  * Opus: validate the header in Decoder::from_head (#425) by @MarekSpodBiedry in https://github.com/storytold/deckcraft/pull/442
  * xtask: assets audit no longer panics on Unicode names and reads real paths from git (#424) by @MarekSpodBiedry in https://github.com/storytold/deckcraft/pull/443
  * Info: keep a property draft when another field or the side menu is clicked (#422) by @MarekSpodBiedry in https://github.com/storytold/deckcraft/pull/445
  * Engine: moving the selection to another text body settles the composition (#421) by @MarekSpodBiedry in https://github.com/storytold/deckcraft/pull/446

## 0.7.1

- Upstream deckcraft v0.7.0.
  ## What's Changed
  * Text: superscript and subscript letters no longer spread apart by @echelon in https://github.com/storytold/deckcraft/pull/264
  * Themes: background pictures are recoloured, and dark layouts stay dark by @echelon in https://github.com/storytold/deckcraft/pull/269
  * Render: two-colour gradients ramp like PowerPoint's by @echelon in https://github.com/storytold/deckcraft/pull/273
  * PPTX: a picture fill with no stretch or tile is tiled, as in PowerPoint by @echelon in https://github.com/storytold/deckcraft/pull/290

## 0.7.0

- Upstream deckcraft v0.7.0.
  ## What's Changed
  * Text: superscript and subscript letters no longer spread apart by @echelon in https://github.com/storytold/deckcraft/pull/264
  * Themes: background pictures are recoloured, and dark layouts stay dark by @echelon in https://github.com/storytold/deckcraft/pull/269
  * Render: two-colour gradients ramp like PowerPoint's by @echelon in https://github.com/storytold/deckcraft/pull/273
  * PPTX: a picture fill with no stretch or tile is tiled, as in PowerPoint by @echelon in https://github.com/storytold/deckcraft/pull/290

## 0.6.2

- Upstream deckcraft v0.6.1.
  ## What's Changed
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/deckcraft/pull/152
  * Release workflow: pass the Azure signing secrets by @echelon in https://github.com/storytold/deckcraft/pull/181
  **Full Changelog**: https://github.com/storytold/deckcraft/compare/v0.5.0...v0.6.1

## 0.6.1

- Upstream deckcraft v0.6.1.
  ## What's Changed
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/deckcraft/pull/152
  * Release workflow: pass the Azure signing secrets by @echelon in https://github.com/storytold/deckcraft/pull/181
  **Full Changelog**: https://github.com/storytold/deckcraft/compare/v0.5.0...v0.6.1

## 0.5.0

- Upstream deckcraft v0.5.0.
  ## What's Changed
  * Issue #23: README lists the ALSA headers a Linux build needs by @LloydNicholson in https://github.com/storytold/deckcraft/pull/24
  * Morph test: sort the ids before dedup so it really checks uniqueness by @alexanderalber in https://github.com/storytold/deckcraft/pull/25
  * App icon: scalable SVG for Linux desktops (#12) by @alexanderalber in https://github.com/storytold/deckcraft/pull/29
  * Slide show: transitions show the old slide instead of the new one twice (#33) by @alexanderalber in https://github.com/storytold/deckcraft/pull/46

## 0.4.0

- Initial BifrOSt App Store package of upstream deckcraft v0.4.0.
