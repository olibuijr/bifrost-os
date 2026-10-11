# ExtensionKit parity

Framework: ExtensionKit (official: https://developer.apple.com/documentation/extensionkit, macOS 13.0+).
Matrix row: `docs/parity-matrix/data/extensionkit.json`. Status moves from `gap` to `partial`.

## What was built

`springboard/ui/src/extensions.rs` (`bifrost_ui::extensions`, reachable from apps as `bifrost_app::controls::extensions`).
Pure data, no I/O, std only:

- `Scene`: an extension point, a scene identifier and a title. `Scene::new` validates tokens and rejects an empty title.
- `Extension`: a declared scene together with the app that ships it.
- `Registry::declare` / `remove`: the scenes apps declare. Redeclaring replaces the scene and keeps the user's choice.
- `Registry::set_enabled` / `is_enabled`: the user's choice per (app, identifier). Extensions are off until enabled, and
  the choice is kept even while the scene is not installed.
- `Registry::present(point)`: `Presentation::Remote` with the first enabled scene in declaration order, or
  `Presentation::Placeholder`. This is the decision a host makes before it shows content.
- `Registry::browser(points)`: one `Section` per extension point in the host's order, with scenes sorted by title and
  disabled scenes included. The shell can draw it as the enable and disable list.
- `Registry::to_lines` / `apply_lines`: the choices as `app<TAB>identifier<TAB>1|0` lines for the shell's prefs. Malformed
  lines are skipped.

Tests: 7 behaviour tests in the module (validation, default off, choice outliving an uninstall, redeclaration, present
order, browser grouping and sorting, persistence round trip).

## Apple names this mirrors

- `AppExtensionScene`, `PrimitiveAppExtensionScene`: `Scene` (declaration only, no content type).
- `AppExtensionSceneConfiguration`: `Scene::new` validation. The connection callback `accept(connection:)` is not modelled.
- `EXHostViewController` and its `placeholderView`: `Registry::present` and `Presentation::Placeholder`. Hosting is not modelled.
- `EXAppExtensionBrowserViewController`: `Registry::browser` and the enable and disable choice. Nothing draws it yet.

## Evidence

- REA passive evidence of the rendered official landing page:
  `ev_83e81a134a9831c3a0e03f0aed9321c069fa73c587a75512c90306f82843de7a`
  (`rea inspect-web-page` against the Chromium CDP browser on 127.0.0.1:9333; title "ExtensionKit | Apple Developer Documentation").
- Earlier matrix evidence (kept for reference): `ev_66f2e23d9209aec92885debef8fcd9b97c0d94d942688fc0cf04fa1effd43117`.
- Official DocC structure: the landing page groups symbols into UI definition (`AppExtensionScene`, `PrimitiveAppExtensionScene`,
  `AppExtensionSceneBuilder`), app extension configuration (`AppExtensionSceneConfiguration`) and host app presentation
  (`EXHostViewController`, `EXAppExtensionBrowserViewController`).

## Remaining gaps

- No out-of-process remote view hosting. `ControllerHost` in `springboard/sdk/src/view/app.rs` runs in the owner's process only.
- No drawn enable and disable browser in the shell.
- No scene content type, no builder syntax and no connection acceptance.
- No activation or deactivation delegate (`EXHostViewControllerDelegate`).
- Share sheet extensions supplied by apps (see `docs/bifrost-ui.md`, Share sheet). Share targets still come only from manifest
  handles and schemes.
- Process isolation for extensions remains with the OS sandbox (`status: os`).

Not verified by a build in this round: the module was written without running cargo, as the task required. The next
build and test stage must confirm it compiles.
