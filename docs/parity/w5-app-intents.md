# Wave 5: App Intents (focus filters, app enums)

Branch `parity/w5-app-intents`, on top of `parity/integration` (waves 1 to 4).

## What was built

- `springboard/ui/src/focus_filter.rs` (new, `pub mod focus_filter;` in `springboard/ui/src/lib.rs`): a pure model of
  focus filters. `AppContext` holds the key and value text an app stores for one Focus. `FocusFilters` keeps one
  context per Focus name, the Focus that is on (`activate`, `active`), `current` (the context of the active Focus),
  `remove`, `invalidate` and a `generation` counter that changes only on real changes. It is not wired into the
  shell: `springboard/src/shell/focus.rs` is still Do Not Disturb only, so nothing switches Focus by name yet.
- App enums (matrix row "App enums") were already in `springboard/sdk/src/intents.rs` (`ParamType::Enum`, with
  `enum(a|b)` parsing, validation and manifest text, and tests at the end of the file). This wave did not change that
  code. The matrix row was still `gap`, so it is closed here as `partial`.

## Official API names mirrored

- `SetFocusFilterIntent` (protocol), `current`, `suggestedFocusFilters(for:)` (not built), `appContext`,
  `invalidateFocusFilterAppContext()`. Source: https://developer.apple.com/documentation/appintents/setfocusfilterintent
- `AppEnum`, `defaultResolverSpecification`, `EnumURLRepresentation` (not built). Source:
  https://developer.apple.com/documentation/appintents/appenum

Gaps that remain: no suggested focus configurations, no Focus switching by the shell, no `AppEnum` display
representations per case (`TypeDisplayRepresentable`, `CaseDisplayRepresentable`), no enum URL representation.

## REA evidence

- SetFocusFilterIntent page: `ev_b90a69827b3a719331975b3ac1f8f042cddb86e074ba4fdde9dc0900a0469525`
- AppEnum page: `ev_6dc907addf560c5506fc4c0c034e7ea505c618aac2b51040db1dc89543d9ea94`

## Matrix rows closed

| framework slug | feature | new status | BifrOSt path |
|---|---|---|---|
| app-intents | App enums | partial | springboard/sdk/src/intents.rs (ParamType::Enum) |
| app-intents | Focus filters | partial | springboard/ui/src/focus_filter.rs |

Verification: `python3 tools/check-names.py` reports no hits. Per the wave rules, no build or test was run.
