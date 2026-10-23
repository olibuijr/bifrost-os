## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.10.0

- Upstream wordcraft v0.10.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/wordcraft/pull/2935
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/wordcraft/pull/3302

## 0.9.0

- Upstream wordcraft v0.9.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Resolve exact OpenType full font names by @finwarman in https://github.com/storytold/wordcraft/pull/2516
  * Recognize the Symbol PCL6 font alias by @finwarman in https://github.com/storytold/wordcraft/pull/2521

## 0.8.0

- Upstream wordcraft v0.8.0.
  ## What's Changed
  * CI: main passes again (Word 2003 check reads through io; first keystroke after a chrome click isn't dropped) by @dkhlapov in https://github.com/storytold/wordcraft/pull/1578
  * Interface-translation audit green and back in CI (#1128) by @dkhlapov in https://github.com/storytold/wordcraft/pull/1143
  * Double-click the margin to create and edit a header or footer (#960) by @dkhlapov in https://github.com/storytold/wordcraft/pull/1144
  * Alt+numpad character codes (#961) by @dkhlapov in https://github.com/storytold/wordcraft/pull/1146

## 0.7.1

- Upstream wordcraft v0.7.0.
  ## What's Changed
  * Font list previews no longer keep every family in memory (#848) by @dkhlapov in https://github.com/storytold/wordcraft/pull/874
  * Layout: blank pages put sections on the right side, and paragraph spacing is the larger of after and before (#733, rebased) by @echelon in https://github.com/storytold/wordcraft/pull/964
  * Insert › Picture takes WMF and EMF metafiles (#802) by @dkhlapov in https://github.com/storytold/wordcraft/pull/869
  * Wingdings, Webdings and Symbol text draws as Unicode symbols (#776) by @dkhlapov in https://github.com/storytold/wordcraft/pull/876

## 0.7.0

- Upstream wordcraft v0.7.0.
  ## What's Changed
  * Font list previews no longer keep every family in memory (#848) by @dkhlapov in https://github.com/storytold/wordcraft/pull/874
  * Layout: blank pages put sections on the right side, and paragraph spacing is the larger of after and before (#733, rebased) by @echelon in https://github.com/storytold/wordcraft/pull/964
  * Insert › Picture takes WMF and EMF metafiles (#802) by @dkhlapov in https://github.com/storytold/wordcraft/pull/869
  * Wingdings, Webdings and Symbol text draws as Unicode symbols (#776) by @dkhlapov in https://github.com/storytold/wordcraft/pull/876

## 0.6.2

- Upstream wordcraft v0.6.1.
  ## What's Changed
  * Hyperlinks cover only their own text (#413) by @vangel76 in https://github.com/storytold/wordcraft/pull/449
  * Groups keep rotated members' orientation; group resize; ink erase after rotation (#441, #442, #443, #445) by @vangel76 in https://github.com/storytold/wordcraft/pull/474
  * Mail merge: SKIPIF, NEXT, IF in preview, header/footer fields (#427, #428, #429, #430) by @vangel76 in https://github.com/storytold/wordcraft/pull/478
  * Options: interface size setting (#475) by @vangel76 in https://github.com/storytold/wordcraft/pull/480

## 0.6.1

- Upstream wordcraft v0.6.1.
  ## What's Changed
  * Hyperlinks cover only their own text (#413) by @vangel76 in https://github.com/storytold/wordcraft/pull/449
  * Groups keep rotated members' orientation; group resize; ink erase after rotation (#441, #442, #443, #445) by @vangel76 in https://github.com/storytold/wordcraft/pull/474
  * Mail merge: SKIPIF, NEXT, IF in preview, header/footer fields (#427, #428, #429, #430) by @vangel76 in https://github.com/storytold/wordcraft/pull/478
  * Options: interface size setting (#475) by @vangel76 in https://github.com/storytold/wordcraft/pull/480

## 0.5.1

- Upstream wordcraft v0.5.0.
  ## What's Changed
  * Typing parity with Word: lists, AutoFormat and AutoCorrect as you type by @echelon in https://github.com/storytold/wordcraft/pull/204
  * Desktop: keep the off-screen check until the monitor size is known (#23) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/76
  * DOCX: keep a line break written directly in a paragraph (#96) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/101
  * Page break (Ctrl+Enter) survives ODT/TXT/Markdown/HTML export and no longer glues words (#84) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/86

## 0.5.0

- Upstream wordcraft v0.5.0.
  ## What's Changed
  * Typing parity with Word: lists, AutoFormat and AutoCorrect as you type by @echelon in https://github.com/storytold/wordcraft/pull/204
  * Desktop: keep the off-screen check until the monitor size is known (#23) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/76
  * DOCX: keep a line break written directly in a paragraph (#96) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/101
  * Page break (Ctrl+Enter) survives ODT/TXT/Markdown/HTML export and no longer glues words (#84) by @MarekSpodBiedry in https://github.com/storytold/wordcraft/pull/86

## 0.4.0

- Initial BifrOSt App Store package of upstream wordcraft v0.4.0.
