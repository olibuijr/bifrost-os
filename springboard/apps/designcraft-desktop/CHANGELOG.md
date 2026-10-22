## 0.5.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.11.0

- Upstream designcraft v0.11.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/designcraft/pull/1266
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/designcraft/pull/1386

## 0.9.0

- Upstream designcraft v0.9.0.
  ## What's Changed
  * Swatches panel header like InDesign's (Fill/Stroke proxy, container/text, Tint); swatches go to the fill or stroke in front (blocked by #592) by @moppeeer in https://github.com/storytold/designcraft/pull/838
  * Object › Fitting with InDesign's shortcuts (Fill Frame Proportionally Ctrl+Alt+Shift+C, …) by @moppeeer in https://github.com/storytold/designcraft/pull/840
  * Respect Find/Change scope and first-match limits by @sanyamsingh001 in https://github.com/storytold/designcraft/pull/845
  * fix: repaint canvas when entering and leaving parent editing by @harrisonoconnorhover in https://github.com/storytold/designcraft/pull/855

## 0.8.1

- Upstream designcraft v0.8.0.
  ## What's Changed
  * Files: saves and exports replace the file atomically by @aiongg in https://github.com/storytold/designcraft/pull/485
  * Recovery: each running app keeps its own recovery data by @noah1400 in https://github.com/storytold/designcraft/pull/506
  * UI: quitting asks to save documents with unsaved changes by @aiongg in https://github.com/storytold/designcraft/pull/545
  * fix(save): preserve live state when persistence fails by @skylartaylor in https://github.com/storytold/designcraft/pull/613

## 0.8.0

- Upstream designcraft v0.8.0.
  ## What's Changed
  * Files: saves and exports replace the file atomically by @aiongg in https://github.com/storytold/designcraft/pull/485
  * Recovery: each running app keeps its own recovery data by @noah1400 in https://github.com/storytold/designcraft/pull/506
  * UI: quitting asks to save documents with unsaved changes by @aiongg in https://github.com/storytold/designcraft/pull/545
  * fix(save): preserve live state when persistence fails by @skylartaylor in https://github.com/storytold/designcraft/pull/613

## 0.7.2

- Upstream designcraft v0.7.1.
  ## What's Changed
  * Adopt shared panel docking and preserve workspace commands by @skylartaylor in https://github.com/storytold/designcraft/pull/409
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/designcraft/pull/489
  * Place: PDF pages and SVGs keep their exact size by @japbcoelho in https://github.com/storytold/designcraft/pull/511
  * Hyphenation: Portuguese text is hyphenated with hyph-utf8's Portuguese patterns by @japbcoelho in https://github.com/storytold/designcraft/pull/512

## 0.7.1

- Upstream designcraft v0.7.1.
  ## What's Changed
  * Adopt shared panel docking and preserve workspace commands by @skylartaylor in https://github.com/storytold/designcraft/pull/409
  * ci: refactor workflows to use storytold/craft-common-actions by @dylanmtaylor in https://github.com/storytold/designcraft/pull/489
  * Place: PDF pages and SVGs keep their exact size by @japbcoelho in https://github.com/storytold/designcraft/pull/511
  * Hyphenation: Portuguese text is hyphenated with hyph-utf8's Portuguese patterns by @japbcoelho in https://github.com/storytold/designcraft/pull/512

## 0.6.1

- Upstream designcraft v0.6.0.
  ## What's Changed
  * Images: PSD decode passes clippy on Rust 1.98 by @jahala in https://github.com/storytold/designcraft/pull/136
  * Compose: don't panic when a hyphenated paragraph starts with a no-break letter by @IHAGI-c in https://github.com/storytold/designcraft/pull/115
  * Show an alert when a file can't be opened or placed by @aiongg in https://github.com/storytold/designcraft/pull/80
  * UI: text fields take Select All, Copy, Cut, Paste, Undo and Redo from the Mac menu (#30) by @aiongg in https://github.com/storytold/designcraft/pull/111

## 0.6.0

- Upstream designcraft v0.6.0.
  ## What's Changed
  * Images: PSD decode passes clippy on Rust 1.98 by @jahala in https://github.com/storytold/designcraft/pull/136
  * Compose: don't panic when a hyphenated paragraph starts with a no-break letter by @IHAGI-c in https://github.com/storytold/designcraft/pull/115
  * Show an alert when a file can't be opened or placed by @aiongg in https://github.com/storytold/designcraft/pull/80
  * UI: text fields take Select All, Copy, Cut, Paste, Undo and Redo from the Mac menu (#30) by @aiongg in https://github.com/storytold/designcraft/pull/111

## 0.5.0

- Initial BifrOSt App Store package of upstream designcraft v0.5.0.
