# Parity wave 4: WidgetKit

Three pure models in `springboard/ui/src` close three WidgetKit rows that were gaps or OS-only. They are unit-tested
host models: nothing wires them into the shell or into apps yet, and nothing has run on the VM or Midget.

## What was built

| Module | Mirrors (official names) | What it does |
|---|---|---|
| `springboard/ui/src/widget_kind.rs` | `Widget`, `WidgetBundle`, `WidgetFamily`, `supportedFamilies(_:)` | A widget kind has a unique identifier (the string a widget is reloaded by), a display name and description, the sizes it supports (small, medium, large), and a configurable flag. `WidgetBundle` holds kinds with unique identifiers, filters kinds by size, and lists the sizes any kind supports. |
| `springboard/ui/src/widget_config.rs` | `AppIntentConfiguration` user-configurable options, `WidgetInfo` (configured values) | Choice, flag and bounded-number options, each with a default. Validates definitions (unique names, non-empty choices, defaults in range) and every value set (kind and range). Reports the value in effect, the choice name, and supports reset. |
| `springboard/ui/src/widget_date.rs` | Dynamic dates in widgets (relative and timer date text) | Text for a target date at a moment (relative wording reuses `format::relative`; clock wording reuses `format::duration`), and the instant the text can next change, so a timeline asks for the next entry in time. |

`WidgetFamily` in the official reference also lists extra-large and accessory sizes. These are not modelled: the
matrix row for them is `n/a` (Watch, Lock Screen and visionOS hardware BifrOSt does not ship).

Ids are ASCII letters, digits, `.`, `-`, `_`, up to 128 bytes. Relative wording in `widget_date` is BifrOSt's own
wording, not Apple's text.

## Official API names mirrored

- WidgetKit landing: `Widget`, `WidgetBundle`, `WidgetFamily`, `WidgetCenter`, `AppIntentConfiguration`
- `AppIntentConfiguration`: `init(kind:intent:provider:content:)`, `supportedFamilies(_:)`, `configurationDisplayName(_:)`, `description(_:)`
- `WidgetFamily`: `systemSmall`, `systemMedium`, `systemLarge` (extra-large and accessory families out of scope)

## REA evidence

- WidgetKit landing page: `ev_df0410e6a60ee92f1c31298255f5b4f64e37f5138f10348bf710f4114434880d`
- `WidgetFamily`: `ev_df3f95bb5597ede97ae437ab383d467c3cac8fac2f82819b22f51c084d7690a8`
- `AppIntentConfiguration`: `ev_1ded0fe4a691815e40082a3bcfac6bac7b3318c912a7f6e17825d7692949c63d`
- "Displaying dynamic dates in widgets": `ev_90b9dbd5f91b23e932a8dfb9346a93dcc9ad85b4786955f2e2c279b713699808`.
  The rendered page was a Developer "page not found" response, so the Apple date-style wording was not verified.
  The `widget_date` rules are BifrOSt's own refresh model and are described as such in the module.

The WidgetKit `WidgetBundle` and `Widget` symbol pages returned 404 from the DocC source during this run; those names
are cited from the landing page's topic list and its REA evidence only.

## Matrix rows closed

| Framework | Feature (as in `docs/parity-matrix/data/widgetkit.json`) | New status | BifrOSt path |
|---|---|---|---|
| WidgetKit | Widget extension and widget configuration (Widget, WidgetBundle, StaticConfiguration) | partial | `springboard/ui/src/widget_kind.rs` |
| WidgetKit | Configurable widgets (AppIntentConfiguration, WidgetInfo) | partial | `springboard/ui/src/widget_config.rs` |
| WidgetKit | Dynamic dates and clock content in widgets | partial | `springboard/ui/src/widget_date.rs` |

## Still open

- Nothing calls these models from the shell (`springboard/src/shell/widgets/mod.rs`) or from the SDK. Apps still
  cannot register a kind.
- Shell widget sizes are still small and medium (`SMALL_PT`, `MEDIUM_PT`); the large family is only in the model.
- No test runs yet: the commit was made without builds, per the wave rules. The next integration run must compile
  `bifrost-ui` and run its tests.
