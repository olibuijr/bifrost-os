# TipKit parity: contextual tips (pure model)

Status after this round: four TipKit gaps closed to `partial`. The tip model, display rules, tip groups and testing
hooks exist as a renderer-independent model in `springboard/ui/src/tips.rs` (bifrost-ui). No SDK view draws a tip yet.

## What was built

`springboard/ui/src/tips.rs`, one `pub mod tips;` line in `springboard/ui/src/lib.rs`.

- `Tip` (trait): `id`, `title`, `message`, `actions`, `rules`. The id is the datastore key and the testing handle.
  There is no image field yet.
- `Rule` (enum): `EventCount { event, at_least }` (an event donated at least N times) and
  `Parameter { name, equals }` (a parameter holds an equal value). `Value` holds `Bool`, `Int` or `Str`.
- `Datastore`: event counts (`donate`, `event_count`), parameters (`set_parameter`, `parameter`), shown counts and
  the last shown time (`mark_shown`, `shown_count`), invalidation for acted-on or dismissed tips (`invalidate`,
  `is_invalidated`), `reset_datastore`, and `is_eligible(tip, now)`.
- `configure(&[ConfigOption]) -> Result<Datastore, TipError>`: `ConfigOption::DisplayFrequency` and
  `ConfigOption::DatastorePath`. A missing file is an empty store. A file that exists but cannot be read is an error.
- `DisplayFrequency`: `Immediate`, `Hourly`, `Daily`, `Weekly`, `Monthly`. The gap after a shown tip is counted from
  the last shown tip, not per tip.
- `TipGroup` with `Priority::FirstAvailable` (the first eligible tip) and `Priority::Ordered` (the first unsettled
  tip, shown only when eligible). `current(&store, now)` returns the index to show.
- Testing: `show_all_for_testing`, `hide_all_for_testing`, `show_for_testing`, `hide_for_testing`, `clear_testing`.
- Persistence: one tab-separated record per line, with `\t`, `\n`, `\r` and `\\` escaped. The file is written to a
  temporary sibling and renamed over the target. Every mutator saves when a path is set, and malformed lines are
  skipped on load.

## Official API names mirrored

Official pages: `https://developer.apple.com/documentation/tipkit` and its sub-pages `tip`, `tipgroup`, `rule`,
`event`, `parameter`.

- `Tip` (protocol) -> `tips::Tip`
- `TipGroup` and `TipGroup.Priority` (`firstAvailable`, `ordered`) -> `tips::TipGroup`, `tips::Priority`
- `Tips.configure(_:)`, `Tips.datastoreLocation(_:)`, `Tips.displayFrequency(_:)` -> `tips::configure`,
  `ConfigOption::DatastorePath`, `ConfigOption::DisplayFrequency`
- `Tips.Rule`, `Tips.Parameter`, `Tips.Event` -> `Rule`, `Datastore::set_parameter`, `Datastore::donate`
- `Tips.resetDatastore()` -> `Datastore::reset_datastore`
- `Tips.showAllTipsForTesting()`, `showTipsForTesting(_:)`, `hideAllTipsForTesting()`, `hideTipsForTesting(_:)` ->
  the `*_for_testing` methods

Design choices that are ours, not from the docs:

- A hide override beats a show override, and `hide_all` beats `show_all`.
- A show override skips the rules and the display frequency. It does not override a hide.
- The default display frequency is `Immediate`.
- `Datastore::invalidate` is the model of "the person acted on or dismissed the tip". The shell calls it on the
  tip's action or close control.

## REA evidence

Recorded with `tools/parity/rea-page.sh` against the rendered official pages:

- TipKit landing page: `ev_8a4a5a10b01dc395c8b8ee1189385dc325cd707594f9dd2c154f88f0b71b5e63`
- `tipkit/tip`: `ev_0a7f344bd5eea439375c8356131b006b04906ed797b83db4f205c6959442a72d`
- `tipkit/tipgroup`: `ev_0df61f5b13c04011547364047736c7edd0ad65e2c4c781591478ad6c714c2989`
- `tipkit/rule`: `ev_b260488f542c8393462070d787589fd1235da869a1c989414a6d76dc6a5c8f88`
- `tipkit/event`: `ev_ff30675058c2a67d5adb34b2fce5066ade56ca0e640fd7803194470198cc67d2`
- `tipkit/parameter`: `ev_8e4f0adfc46dbd4f7162ca6285ae0adfa8d96a756a9df1efc42ec32ca8d05290`

## Matrix rows closed (`docs/parity-matrix/data/tipkit.json`)

| Framework | Feature (as written in the matrix) | New status | BifrOSt path |
|---|---|---|---|
| TipKit | Tip protocol (title, message, image, actions) | partial | `springboard/ui/src/tips.rs` (`Tip`, `Action`; no image) |
| TipKit | Display rules and events | partial | `springboard/ui/src/tips.rs` (`Rule`, `Datastore::donate`, `set_parameter`) |
| TipKit | Tip groups | partial | `springboard/ui/src/tips.rs` (`TipGroup`, `Priority`) |
| TipKit | Configuration and datastore | partial (extended: configure, display frequency, file datastore; still no CloudKit) | `springboard/ui/src/tips.rs` (`configure`, `Datastore`) |
| TipKit | Testing hooks (show, hide, reset tips) | partial | `springboard/ui/src/tips.rs` (`*_for_testing`, `reset_datastore`) |

Rows left unchanged: inline tip view (gap), popover tip (partial, no tip content yet), tooltips (partial), CloudKit
sync (n/a), and the UIKit and AppKit tip views (n/a).

## Still missing

- No SDK wiring: no `bifrost_app` call that draws a tip, and no anchor modifier that opens a popover tip from a view.
- No inline tip view or tip view style.
- No tip image, and no `Tip.Option`-style per-tip options (such as a maximum display count).
- The Datastore is not wired to the app's data directory. An app must pass `ConfigOption::DatastorePath` itself.

## Verification

Not built or run, per the brief. Name guard: `python3 tools/check-names.py` exits 0. The module has 9 behaviour tests
in `tips::tests`.
