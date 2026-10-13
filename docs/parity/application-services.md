# Application Services: parity work (round 1)

Official framework page: https://developer.apple.com/documentation/applicationservices
Matrix row file: `docs/parity-matrix/data/application-services.json`

## What was built

Three gaps from the matrix were taken in pure Rust with std only. Each is a model in `springboard/ui/src` (bifrost-ui); none
is wired to the accessibility bridge or the SDK yet, and none has been built or run in this round.

1. **Region (shape) type**: `springboard/ui/src/geometry.rs`, `Region`.
   A set of disjoint rectangles with `from_rect`, `rects`, `is_empty`, `contains`, `bounds`, `area`, `translated`, and the
   boolean operations `union`, `intersection`, `subtract` and `xor`. Each operation sweeps vertical slabs cut at every edge and
   joins neighbouring pieces that share an edge, so results stay small. Tests: `a_region_from_a_rectangle...`,
   `union_covers_both_once...`, `subtract_leaves...`, `neighbouring_pieces...`, `translation_moves...`.
   Mirrors: `HIShape` and `HIMutableShape` (Application Services). The official page for each was fetched; the DocC payload for
   the method list was empty, so only the type names are claimed here. Curved paths are not modelled.

2. **Announcement queue** (live-region text): `springboard/ui/src/ax.rs`, `Announcements`, `Politeness`, `Announcement`.
   Polite and assertive priority, the oldest assertive item taken first, blank text and an immediate repeat refused, whitespace
   collapsed, and a bounded queue that drops the oldest polite item when full. Tests:
   `an_assertive_announcement_is_spoken...`, `blank_and_repeated...`, `a_full_queue_drops...`.
   Mirrors: the announcement-requested notification, its announcement key and its priority key (`AXNotificationConstants.h`).
   Not yet done: the AT-SPI bridge does not read the queue, so nothing is spoken.

3. **Named custom actions per element**: `springboard/ui/src/ax.rs`, `Map::add_custom_action`, `Map::custom_actions`,
   `Map::has_custom_action`.
   Actions beyond press, stored per element address. Refused for an unknown element, a blank name, or a duplicate (case-insensitive).
   Tests: `custom_actions_belong_to_one_element...`.
   Mirrors: `NSAccessibilityCustomAction` (name, handler) and the `accessibilityActionNames` family (deprecated, so only the
   concept is mirrored). Not yet done: the bridge does not list them and no assistive-technology invocation path exists.

Owner-rule check: `python3 tools/check-names.py --summary` reports 0 hits after these changes.

## Official sources

- `python3 -I tools/parity/docc.py applicationservices`: the framework's managers, the Carbon Accessibility reference
  headers, and the class list (`HIMutableShape`, `HIShape`, the ColorSync classes).
- `docc.py applicationservices/himutableshape` and `applicationservices/hishape`: type pages, macOS 10.2.
- `docc.py appkit/nsaccessibilitycustomaction`: name, handler, target, selector, macOS 10.13.
- `docc.py appkit/nsaccessibility`: the legacy informal protocol; its action and attribute methods are deprecated.
- `docc.py applicationservices/axnotificationconstants_h --depth 2`: the announcement-requested notification, the announcement key,
  and the priority key values.

## REA evidence

- `ev_a6c69eba463253162e20cfff69c1bf598c5e97022d7ad862ba4cecf4f4f98f9b`: rendered official landing page
  (`rea-page.sh https://developer.apple.com/documentation/applicationservices`, title "Application Services | Apple Developer Documentation").
- `ev_aa67df2e51e2916455b18b5705ec62cb03dcae34a55905e84b9fc3290a9c1ff9`: the evidence id the matrix already carried for this framework.

## Remaining gaps

- Accessibility text: selected text, visible range, text markers and caret events (unchanged).
- Announcements and custom actions are models only; the AT-SPI bridge must read the queue and list and invoke the actions, and this
  needs a live Orca check before the row can be `parity`.
- Region: no curved paths, and no `bifrost_app` wrapper yet.
- ColorSync, speech synthesis, print-plug-in and zoom focus APIs: no work in this round.
- Core Printing pagination (`PMGetAdjustedPageRect` equivalent): not attempted; the official Core Printing page path did not resolve
  through the DocC helper.

## Verification

No builds, tests or cargo runs were made in this round, as the task required. The new tests are listed above and must be run by the
build stage (`cargo test` for `bifrost-ui`).

## ColorSync Manager row (team A review, parity/team-a-10)

The row "ColorSync Manager (profiles, transforms, CMM)" moves from gap to partial. It points at code that already
exists: `ColorSpace::from_icc` and `ColorConversionInfo` in `springboard/render/src/cg/color.rs`, with the profile header
and tag reader in `springboard/render/src/icc.rs`. An earlier draft on this branch added a second ICC reader in
`springboard/ui/src/colour_profile.rs`; it was removed because the render crate already covers it (and also handles
parametric curve types 0 to 4, the white point tag and rendering intents). There is still no SDK profile API for apps, no
LUT-based profile support and no CMM. Not yet verified on Midget.
