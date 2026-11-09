## 0.8.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.22.0

- Upstream vectorcraft v0.22.0.
  ## What's Changed
  * Affinity import: bound archive index names and charge failed entry extraction by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/5291
  * Native format: document.save can write path coordinates at a fixed precision by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/3990
  * Drop invalid XML characters from SVG link URLs and aria-labels by @Silvertree2010 in https://github.com/storytold/vectorcraft/pull/5279
  * Fix #5312: EPS files written by Ghostscript's eps2write open by @JonRC in https://github.com/storytold/vectorcraft/pull/5314

## 0.21.1

- Upstream vectorcraft v0.21.1.
  **Full Changelog**: https://github.com/storytold/vectorcraft/compare/v0.21.0...v0.21.1

## 0.21.0

- Upstream vectorcraft v0.21.0.
  ## What's Changed
  * Paste: in front or back of a compound path's member, other art goes beside the compound path by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/5120
  * .ai import: version 7 palettes keep their swatches without names by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/5123
  * Variables: Rename reports a missing variable or data set before a taken name by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/5135
  * Variables: a graph variable's data set applies its CSV as graph.setData does by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/5136

## 0.20.0

- Upstream vectorcraft v0.20.0.
  ## What's Changed
  * Updates: shared check-result dialog from craft-update by @aiongg in https://github.com/storytold/vectorcraft/pull/4814
  * docs: tidy agent, contributor and asset docs by @aiongg in https://github.com/storytold/vectorcraft/pull/4807
  * Shortcuts: in the web build a key-position shortcut uses the key of the same press by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/4867
  * Fix #4864: a search pick of an Asset Export command opens the panel by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/4869

## 0.19.0

- Upstream vectorcraft v0.19.0.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/4482
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/4487
  * Enforce Rust coding standards via Clippy by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/4299
  * Remove the experimental Windows 7 build by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/4497

## 0.18.0

- Upstream vectorcraft v0.18.0.
  ## What's Changed
  * Reject exponent notation in the pica form of Unit::parse by @Silvertree2010 in https://github.com/storytold/vectorcraft/pull/4126
  * Refuse signs, exponents and non-numeric text in Export for Screens sizes by @Silvertree2010 in https://github.com/storytold/vectorcraft/pull/4123
  * Reject misplaced or excess '=' padding in base64_decode by @Silvertree2010 in https://github.com/storytold/vectorcraft/pull/4131
  * Shortcuts: the Cmd+Escape test expects no binding on Windows by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/4096

## 0.17.0

- Upstream vectorcraft v0.17.0.
  ## What's Changed
  * Brushes: one output budget across tracks, tiles and copies (#3594) by @saad-mughal435 in https://github.com/storytold/vectorcraft/pull/3731
  * Docs: the Shape Builder parity line paraphrases the combine page by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/3679
  * G4: paragraph Auto Leading drives the Leading field, leading nudge and placeholder fill by @bflatastic in https://github.com/storytold/vectorcraft/pull/3733
  * G4: hanging punctuation keeps its amounts under Glyph Scaling and burasagari by @bflatastic in https://github.com/storytold/vectorcraft/pull/3736

## 0.16.0

- Upstream vectorcraft v0.16.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Honor Preferences › Performance › Real-time Drawing and Editing during tool drags by @nrq6993-a11y in https://github.com/storytold/vectorcraft/pull/2942
  * Effects: stacked geometry effects keep to an anchor and line budget (#3578) by @odgrim in https://github.com/storytold/vectorcraft/pull/3695

## 0.15.0

- Upstream vectorcraft v0.15.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * docs: add SonarCloud and CodeRabbit badges to README by @dylanmtaylor in https://github.com/storytold/vectorcraft/pull/2923
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/vectorcraft/pull/3247

## 0.14.0

- Upstream vectorcraft v0.14.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * SVG import: hrefs that need no folder stay as written, and images in a non-absolute folder are read by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/2837
  * MCP: parse errors and overlong lines wait for room in the 64-line queue by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/2776

## 0.13.1

- Upstream vectorcraft v0.13.1.
  ## What's Changed
  * Fix #866: macOS save sheets have a Format menu again by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/2379
  * i18n: add Dutch, Danish, Hindi and Indonesian interfaces by @pechnikov2026-star in https://github.com/storytold/vectorcraft/pull/2393
  * Cursors: 36 more tool states get cursors of their own, listed in cursors.toml by @bobtherobot in https://github.com/storytold/vectorcraft/pull/2334
  * Part of #394: Highlight Alternate Glyphs paints pink behind OpenType substitutions by @Messhias in https://github.com/storytold/vectorcraft/pull/1365

## 0.12.0

- Upstream vectorcraft v0.12.0.
  ## What's Changed
  * ci: add comfortable runtime limits to unbounded jobs by @skylartaylor in https://github.com/storytold/vectorcraft/pull/2077
  * Use shared PDF writing and reading with guarded import by @skylartaylor in https://github.com/storytold/vectorcraft/pull/1894
  * Use the shared PDF writer for export and print composition by @skylartaylor in https://github.com/storytold/vectorcraft/pull/1893
  * Importers: a PDF dictionary value the reader can't parse no longer crashes the nesting check by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/2133

## 0.11.1

- Upstream vectorcraft v0.11.0.
  ## What's Changed
  * Artboards: art flush with an artboard moves with it despite a rounding error by @bflatastic in https://github.com/storytold/vectorcraft/pull/1375
  * Fonts: a character no font covered is looked for again after a font rescan by @bflatastic in https://github.com/storytold/vectorcraft/pull/1376
  * Docs: ROADMAP 2026-10-12 adds #1239–#1374 (community PRs of the day) by @bflatastic in https://github.com/storytold/vectorcraft/pull/1378
  * PDF import: a PDF saved with its editing data opens on its artboard with its bleed by @bflatastic in https://github.com/storytold/vectorcraft/pull/1379

## 0.11.0

- Upstream vectorcraft v0.11.0.
  ## What's Changed
  * Artboards: art flush with an artboard moves with it despite a rounding error by @bflatastic in https://github.com/storytold/vectorcraft/pull/1375
  * Fonts: a character no font covered is looked for again after a font rescan by @bflatastic in https://github.com/storytold/vectorcraft/pull/1376
  * Docs: ROADMAP 2026-10-12 adds #1239–#1374 (community PRs of the day) by @bflatastic in https://github.com/storytold/vectorcraft/pull/1378
  * PDF import: a PDF saved with its editing data opens on its artboard with its bleed by @bflatastic in https://github.com/storytold/vectorcraft/pull/1379

## 0.10.1

- Upstream vectorcraft v0.10.0.
  ## What's Changed
  * #1063: a fill or stroke chosen with characters selected colours just them by @bflatastic in https://github.com/storytold/vectorcraft/pull/1075
  * Fix font size and horizontal scale after text/group scaling by @lollinoo in https://github.com/storytold/vectorcraft/pull/1045
  * Type: Proportional Metrics — full-width glyphs on the font's proportional widths, `palt` (#966) by @imachus in https://github.com/storytold/vectorcraft/pull/1067
  * Object: Align, Move, Arrange and other commands that change nothing record no undo step by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/1060

## 0.10.0

- Upstream vectorcraft v0.10.0.
  ## What's Changed
  * #1063: a fill or stroke chosen with characters selected colours just them by @bflatastic in https://github.com/storytold/vectorcraft/pull/1075
  * Fix font size and horizontal scale after text/group scaling by @lollinoo in https://github.com/storytold/vectorcraft/pull/1045
  * Type: Proportional Metrics — full-width glyphs on the font's proportional widths, `palt` (#966) by @imachus in https://github.com/storytold/vectorcraft/pull/1067
  * Object: Align, Move, Arrange and other commands that change nothing record no undo step by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/1060

## 0.9.0

- Upstream vectorcraft v0.9.0.
  ## What's Changed
  * Fix #861: open .ai files whose non-native art has ASCII85 lines starting with %_ by @rschuerer in https://github.com/storytold/vectorcraft/pull/854
  * PDF import: clip art reaching past the page to the crop box by @fastmikey99 in https://github.com/storytold/vectorcraft/pull/675
  * Use File Open path for native SVG drops by @David-T-Campos in https://github.com/storytold/vectorcraft/pull/774
  * Fonts: missing fonts are listed when a document opens, with Find Fonts and a folder search by @dotCooCoo in https://github.com/storytold/vectorcraft/pull/792

## 0.8.0

- Initial BifrOSt App Store package of upstream vectorcraft v0.8.0.
