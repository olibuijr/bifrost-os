# SwiftUI: App structure and Data and storage (parity)

Framework page: https://developer.apple.com/documentation/swiftui
REA evidence of the framework page: `ev_c9694d28e5b52b454b4265832cf0a84e86b67ecf4344b4a586154c5c4e2f2e39`.

This round adds three pure Rust models in `springboard/ui/src` (bifrost-ui, std only, no new crates). Each mirrors one
official SwiftUI API, and each stays a model until the retained views read it.

## What was built

| Gap closed | Official API mirrored | BifrOSt path | Tests | REA evidence |
|---|---|---|---|---|
| Value-based navigation destinations | `NavigationPath` (`init()`, `append(_:)`, `removeLast(_:)`, `count`, `isEmpty`) | `springboard/ui/src/nav.rs`: `NavigationPath<D>`, `NavigationStack::apply_path` | 4 new in `nav.rs` tests | `ev_0c2e7d8a46781c90d5e31dd43178db68ba649ff2f0db6ff609440955b2e9c65f` |
| Environment values through a hierarchy | `EnvironmentValues` (custom keys set and read per view) | `springboard/ui/src/environment.rs`: `Environment`, `EnvValue` | 6 | `ev_5706db00c80781939f1d9836cf789a7da22a2e7082d47debbf2e6b2896f5f69d` |
| Child-to-parent preference channel | `PreferenceKey` (`defaultValue`, `reduce(value:nextValue:)`) | `springboard/ui/src/preferences.rs`: `PreferenceKey` trait, `Preferences<K>` | 6 | `ev_ed7408815c58135c48cdae65e977331b423243a4d285c4486e3d4ce8d5fbb9f7` |

### Navigation path (`nav.rs`)

- `NavigationPath<D>` is a plain ordered list of destination values. `append`, `remove_last` (clamped), `last`, `iter`,
  `count`, `is_empty`, `Default`, `PartialEq`.
- `NavigationStack::apply_path(&path, build)` pops to the root and pushes one screen per destination, with the title and
  screen that `build` returns. Its return value is the number of screens pushed.
- No type erasure: an app keeps one enum for its destinations. The Apple `codable` representation is not mirrored, since
  there is no state-restoration bridge for paths yet.

### Environment (`environment.rs`)

- `Environment` is a stack of scopes. The root always exists. `enter` and `leave` bracket a child's subtree.
- `set` replaces a key in the current scope. `get` returns the innermost setting. `unset` reveals the outer value.
  `flag` reads a boolean and returns false otherwise.
- `EnvValue` holds bool, int, float and text. The set is closed, so no type-erased keys.
- The `EnvironmentValues` default values (appearance, locale, accessibility flags) are not seeded yet.

### Preferences (`preferences.rs`)

- `PreferenceKey` has an associated `Value`, `default_value()` and `reduce(acc, next)`, the same shape as the official
  protocol.
- `Preferences<K>` keeps one contribution per child node id. Setting again keeps the node's place. `value()` folds the
  contributions in the order they were first set. `changed()` returns the combined value only when it differs from the
  last value reported, starting from the default.

## Official names mirrored

`NavigationPath`, `NavigationPath.count`, `NavigationPath.isEmpty`, `append(_:)`, `removeLast(_:)`, `EnvironmentValues`,
`PreferenceKey`, `defaultValue`, `reduce(value:nextValue:)`, `onPreferenceChange` (as `changed()`).

## Remaining gaps

- The environment and preference models are not wired into the retained views. Appearance and locale are still
  global reads, not per-view values, and there is no propagation through the view tree.
- `NavigationPath` is not bound to a live navigation view, and it is not written to or read from state restoration.
- No `navigationDestination` modifier, no presented-destination bindings.
- Anchor preferences (geometry) have no counterpart; layout is computed by the app.
- The declarative App, Scene and View tree, multi-window `openWindow`, a model store, and desktop widgets stay open
  (see the gaps list in `docs/parity-matrix/data/swiftui-app-data.json`).

## Verification

No build or test run was done in this stage, per the task. The unit tests are written for the next stage, which builds
and runs them (`cargo test -p bifrost-ui`). The owner name guard `python3 tools/check-names.py` reports no hits.
