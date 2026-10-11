# App Intents Testing: BifrOSt parity

Framework doc: https://developer.apple.com/documentation/appintentstesting

## What was built

All code is in `springboard/sdk/src/intents.rs` (std only, no new crates, no unsafe).

| Gap (matrix row) | Built | Where |
|---|---|---|
| Enums (enum parameter type) | `ParamType::Enum(Vec<String>)`, spelled `enum(open|closed)` in `name()`, `parse()` and the manifest text. `accepts()` takes only the declared case names. Case names may not contain `|`, `)` or `"`. | `intents.rs` `ParamType` |
| Dynamic property paths | `PropertyPath` (steps `PathStep::Key` / `PathStep::Index`), parsed from `items[2].title`, `items.2.title` or `value.0`, and resolved with `PropertyPath::resolve` against a `Json` value. `IntentResult::at(path)` reads a field of a result. | `intents.rs` `PropertyPath`, `IntentResult::at` |
| Intent test helpers (typed result from a run) | `intents::run(app, ctx, data_dir, id, params)`: finds the intent by id, checks the parameters against its declaration, performs it and returns the typed `IntentResult`. `IntentResult::from_json` reads a control-socket reply back into the struct, so a test can compare a reply and a direct run. | `intents.rs` `run`, `IntentResult::from_json` |

Unit tests in the same file's `tests` module: `enum_parameter_parses_names_and_accepts_only_its_cases`,
`enum_parameter_validates_and_lands_in_manifest_text`, `property_path_parses_names_indexes_and_rejects_malformed`,
`property_path_resolves_into_nested_results`, `result_reads_back_from_its_json_and_reads_paths`,
`run_performs_by_id_and_checks_parameters_first`.

No code was compiled or run for this change; the build and test stage must run the SDK crate's tests (the
`intents` module) before it is merged.

## Official API names this mirrors

- `AnyAppIntent.run()` and `IntentDefinitions`: `intents::run` (in-process, by id, typed result).
- `AnyAppEnum` / `AppEnumDefinition`: `ParamType::Enum` (case names only; no type identifier, no raw value type).
- `DynamicPropertyPath` (`subscript(_:)`, `subscript(dynamicMember:)`): `PropertyPath` (string form, run-time check).
- `ResolvedIntentResult`: `IntentResult`, with `from_json` and `at` added in this change.

## REA evidence

- `ev_aa9407bd3a501bfe1821013f0787fe5c21fb49af97d4c4a41c99b653cb3d977f`: REA passive capture of the rendered
  landing page https://developer.apple.com/documentation/appintentstesting (title "App Intents Testing | Apple Developer
  Documentation"). It lists the topics used in the matrix: the intent definitions, the result types, the value
  query result, `ViewAnnotation`, the type-erased wrappers (including `AnyAppEnum`) and the supporting property path types.
- Official symbol pages read with `tools/parity/docc.py`: `appintentstesting`, `appintentstesting/dynamicpropertypath`
  and `appintentstesting/anyappenum`.

## Remaining gaps

- No test session object. `run()` runs in the same process; there is no runner, no out-of-process execution and no
  test-target wrapper.
- No value query results or intent value queries (`ResolvedValueQueryResult`, `IntentValueQueryDefinition`).
- No entity annotation or onscreen context (`ViewAnnotation`).
- No transient entities (`AnyTransientAppEntity`) and no enum type identifier or typed raw value (`AnyAppEnum`).
- Property paths are checked at run time, not by type, and there is no keyword-argument intent builder
  (`IntentValuePropertiesCallable`).
- Siri and voice dialog verification is not modelled.
