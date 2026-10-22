## 0.4.1

- Opens as a native Wayland window (from the Dock, Spotlight and the App Store).

# Changelog

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
