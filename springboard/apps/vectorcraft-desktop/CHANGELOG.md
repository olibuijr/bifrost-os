## 0.8.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

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
