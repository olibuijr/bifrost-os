## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

## 0.18.0

- Upstream gridcraft v0.18.0.
  ## What's Changed
  * Sort: the closed Sort by box names the column, as its list does (#3446) by @ZIDAN44 in https://github.com/storytold/gridcraft/pull/3506
  * View: a sheet keeps its own scroll position when sheets move (#3485) by @ZIDAN44 in https://github.com/storytold/gridcraft/pull/3507
  * Selection: Ctrl+Shift+Arrow searches the active cell's line (#3484) by @ZIDAN44 in https://github.com/storytold/gridcraft/pull/3508
  * gridcraft-web: window.gridcraft, the page command API from craft-web-api by @helenkwok in https://github.com/storytold/gridcraft/pull/3524

## 0.17.0

- Upstream gridcraft v0.17.0.
  ## What's Changed
  * Ribbon: Font Size value is centered in its box (fixes #3155) by @kenkanuma in https://github.com/storytold/gridcraft/pull/3161
  * Data validation and conditional formats: anchor relative formulas on the top-left cell of all areas (#3152) by @ljkx in https://github.com/storytold/gridcraft/pull/3173
  * Ribbon test: pin macOS for the AutoSum tooltip, so it passes on a Mac by @Aimarmndz in https://github.com/storytold/gridcraft/pull/3179
  * Charts: decimate area top and baseline at paired horizontal indices by @neilcawse in https://github.com/storytold/gridcraft/pull/3132

## 0.16.1

- Upstream gridcraft v0.16.1.
  **Full Changelog**: https://github.com/storytold/gridcraft/compare/v0.16.0...v0.16.1

## 0.16.0

- Upstream gridcraft v0.16.0.
  ## What's Changed
  * Data: Solver Load/Save Model and Reset All by @prxnvt in https://github.com/storytold/gridcraft/pull/3086
  * Data: Solver Options by @prxnvt in https://github.com/storytold/gridcraft/pull/3087
  * M7: add data tables beneath charts by @campbellcody in https://github.com/storytold/gridcraft/pull/3123
  * Page Layout: Page Setup's page order (Down, then over / Over, then down) by @iluebbe in https://github.com/storytold/gridcraft/pull/3130

## 0.15.0

- Upstream gridcraft v0.15.0.
  ## What's Changed
  * Updates: shared check-result dialog from craft-update by @aiongg in https://github.com/storytold/gridcraft/pull/3013
  * docs: tidy agent, contributor and asset docs by @aiongg in https://github.com/storytold/gridcraft/pull/3012
  * Make the Merge & Center ribbon label part of its split button by @Silvertree2010 in https://github.com/storytold/gridcraft/pull/3024
  * Match Excel for YEARFRAC basis 0, MOD, NUMBERVALUE and PERCENTRANK edge cases by @Harrison-Booth in https://github.com/storytold/gridcraft/pull/2657

## 0.14.2

- Upstream gridcraft v0.14.2.
  ## What's Changed
  * Grant network access so update checks can reach GitHub by @dylanmtaylor in https://github.com/storytold/gridcraft/pull/2841
  * Honor CARGO_BUILD_TARGET in FreeBSD packaging for aarch64 release builds by @dylanmtaylor in https://github.com/storytold/gridcraft/pull/2847
  * Macro interface: Automate › Macros, Macros and Record Macro dialogs, step editor with a debugger by @prxnvt in https://github.com/storytold/gridcraft/pull/478
  * Macro engine: macro tokens, resolved to A1 references when a step runs by @prxnvt in https://github.com/storytold/gridcraft/pull/2151

## 0.13.0

- Upstream gridcraft v0.13.0.
  ## What's Changed
  * Trace arrows stay on the traced cell when the selection moves by @marcoarnulfo in https://github.com/storytold/gridcraft/pull/2481
  * Make the Row Height dialog use points like Excel by @Silvertree2010 in https://github.com/storytold/gridcraft/pull/2524
  * Return None from datetime_from_serial when rounding carries past 9999 by @Silvertree2010 in https://github.com/storytold/gridcraft/pull/2528
  * Keep <scheme val="minor"/> on every body-font font on save by @Silvertree2010 in https://github.com/storytold/gridcraft/pull/2557

## 0.12.1

- Upstream gridcraft v0.12.1.
  ## What's Changed
  * Romanian: translate Options|Editing and 22 newer strings by @vioreltus in https://github.com/storytold/gridcraft/pull/2233
  * Keep '!' in structured-reference columns intact on XLSX save by @Silvertree2010 in https://github.com/storytold/gridcraft/pull/2244
  * ui: a too-wide number in a repeat-fill format shows #### by @pick2510 in https://github.com/storytold/gridcraft/pull/2260
  * run --script: a line's unknown key stops the run (#2256) by @Hamza0101 in https://github.com/storytold/gridcraft/pull/2266

## 0.11.1

- Upstream gridcraft v0.11.1.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * security: enforce HTTPS on curl downloads (SonarCloud S6506) by @dylanmtaylor in https://github.com/storytold/gridcraft/pull/2188
  * Backlog integration: 230 reviewed fixes and reference app behaviour changes by @aiongg in https://github.com/storytold/gridcraft/pull/2226

## 0.10.0

- Upstream gridcraft v0.10.0.
  > [!NOTE]
  > **FreeBSD aarch64 is dropped from this release.** We had to cut releases, and we cannot build this target within the 5-hour CI limit for any of the Craft apps, so we are dropping it for now. This is a known blocker, flagged for a future fix in storytold/craft-common-actions#39. FreeBSD x86_64 and every other platform are included.
  ## What's Changed
  * Skip CodeRabbit reviews on Dependabot PRs by @echelon in https://github.com/storytold/gridcraft/pull/2057
  * ci: switch to shared nix workflow from craft-common-actions by @dylanmtaylor in https://github.com/storytold/gridcraft/pull/1620

## 0.8.0

- Upstream gridcraft v0.8.0.
  ## What's Changed
  * ci: add comfortable runtime limits to unbounded jobs by @skylartaylor in https://github.com/storytold/gridcraft/pull/1418
  * fix(edit): recalculate all changed cells after large-range clears by @harrisonoconnorhover in https://github.com/storytold/gridcraft/pull/1128
  * Formula parser: Excel's 8,192-character limit and postfix nesting (never crash) by @nihilist-narwhal in https://github.com/storytold/gridcraft/pull/1330
  * AutoFilter: Enter in the filter menu applies the filter and closes it by @marcoarnulfo in https://github.com/storytold/gridcraft/pull/1588

## 0.7.1

- Upstream gridcraft v0.7.0.
  ## What's Changed
  * fix(calc): recalculate dependents of a cleared spill past 65536 cells (#558) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/605
  * Return #VALUE! when a LAMBDA call has too few argument positions (#557) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/606
  * SUBTOTAL 109-111 leave out nested subtotals like 1-11 (#556) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/607
  * fix(xlsx): keep leading and trailing whitespace in document properties on reopen (#554) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/608

## 0.7.0

- Upstream gridcraft v0.7.0.
  ## What's Changed
  * fix(calc): recalculate dependents of a cleared spill past 65536 cells (#558) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/605
  * Return #VALUE! when a LAMBDA call has too few argument positions (#557) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/606
  * SUBTOTAL 109-111 leave out nested subtotals like 1-11 (#556) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/607
  * fix(xlsx): keep leading and trailing whitespace in document properties on reopen (#554) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/608

## 0.6.2

- Upstream gridcraft v0.6.1.
  ## What's Changed
  * selection.move: clamp extreme dr/dc to the sheet edge instead of overflowing by @webdevtodayjason in https://github.com/storytold/gridcraft/pull/232
  * CHOOSE: pick element by element when the index is an array by @nihilist-narwhal in https://github.com/storytold/gridcraft/pull/252
  * Add MacJedi42 to the contributor names by @MacJedi42 in https://github.com/storytold/gridcraft/pull/253
  * Sort and Remove Duplicates: compare text the way Excel does by @nihilist-narwhal in https://github.com/storytold/gridcraft/pull/255

## 0.6.1

- Upstream gridcraft v0.6.1.
  ## What's Changed
  * selection.move: clamp extreme dr/dc to the sheet edge instead of overflowing by @webdevtodayjason in https://github.com/storytold/gridcraft/pull/232
  * CHOOSE: pick element by element when the index is an array by @nihilist-narwhal in https://github.com/storytold/gridcraft/pull/252
  * Add MacJedi42 to the contributor names by @MacJedi42 in https://github.com/storytold/gridcraft/pull/253
  * Sort and Remove Duplicates: compare text the way Excel does by @nihilist-narwhal in https://github.com/storytold/gridcraft/pull/255

## 0.5.1

- Upstream gridcraft v0.5.0.
  ## What's Changed
  * Desktop: write log records to standard error and a rotating log file by @switch87 in https://github.com/storytold/gridcraft/pull/8
  * Default Windows startup to DirectX 12 by @DWAA1660 in https://github.com/storytold/gridcraft/pull/22
  * Fix: inserting or deleting rows, columns or cells cancels copy/cut mode (#21) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/34
  * Fix missing dropdown arrows on large ribbon buttons (#26) by @harrisonoconnorhover in https://github.com/storytold/gridcraft/pull/35

## 0.5.0

- Upstream gridcraft v0.5.0.
  ## What's Changed
  * Desktop: write log records to standard error and a rotating log file by @switch87 in https://github.com/storytold/gridcraft/pull/8
  * Default Windows startup to DirectX 12 by @DWAA1660 in https://github.com/storytold/gridcraft/pull/22
  * Fix: inserting or deleting rows, columns or cells cancels copy/cut mode (#21) by @MarekSpodBiedry in https://github.com/storytold/gridcraft/pull/34
  * Fix missing dropdown arrows on large ribbon buttons (#26) by @harrisonoconnorhover in https://github.com/storytold/gridcraft/pull/35

## 0.4.0

- Initial BifrOSt App Store package of upstream gridcraft v0.4.0.
