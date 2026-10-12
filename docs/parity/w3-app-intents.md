# Parity wave 3: App Intents

Branch `parity/w3-app-intents`, built on `parity/integration` (waves 1 and 2). Pure Rust, std only, no builds run
in this wave.

## What was built

All changes are in `springboard/sdk/src/intents.rs` (`bifrost_app::intents`).

- **Result views (`IntentResult.Snippet`).** A new `Snippet` model: a title and an ordered list of `SnippetRow`
  values (`label`, `text`). `IntentResult::with_snippet` attaches one. `to_json` writes a `snippet` key only when a
  snippet is set (`null` or absent otherwise). `IntentResult::from_json` reads it back and rejects a malformed
  snippet (a row without label or text, or no title). The shell draws the rows; the app builds the model only.
- **Doubles parameter type.** `ParamType::Double` (`double` in the manifest and in `intent.list`). It accepts finite
  JSON numbers only.
- **Currencies parameter type.** `ParamType::Currency` (`currency`). It accepts an object
  `{"amount": <finite number>, "code": "<three ASCII capitals>"}`. Negative amounts are allowed (refunds).

The enum parameter type (`App enums`) and the execution-mode and authentication model were already in
`intents.rs` from wave 2 and are not counted here. The matrix still lists `App enums` as gap; the lead should
reconcile that row when importing this wave.

## Apple API names mirrored

- `IntentResult` (`result(dialog:view:)`, `Snippet` associated type): https://developer.apple.com/documentation/appintents/intentresult
- `IntentResult.Snippet`: https://developer.apple.com/documentation/appintents/intentresult/snippet
- `IntentParameter` (primitive types `Doubles`; common framework types `Currencies`): https://developer.apple.com/documentation/appintents/intentparameter

Not mirrored: the Swift view builder (`content:` and `view:` closures), `SnippetIntent` (an interactive snippet
that runs an intent), `result(snippetIntent:)`, and the `Currencies` value type with its currency formatting.

## REA evidence

- `ev_437fa4db6c8b5ec6f244dd9fa38bd2a0e81729b78cd9449b3cd750ea360281ed`: rendered page `IntentResult`, shows the
  `Snippet` associated type and the `result(dialog:view:)` family.
- `ev_2ed15c88778ea3fdf2c306d3c2502f5cfaa5b482efb11bd9ea144fbdfe34dea9`: rendered page `IntentParameter`, lists
  the `Doubles` and `Currencies` topics.

## Matrix rows closed

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| app-intents | Snippets and result views | partial | springboard/sdk/src/intents.rs (`Snippet`, `SnippetRow`, `IntentResult::with_snippet`) |
| app-intents | Common data types | partial | springboard/sdk/src/intents.rs (`ParamType::Double`, `ParamType::Currency`) |

Left as gap: Focus filters (no declaration API; `springboard/src/shell/focus.rs` is Do Not Disturb only),
Widgets, controls and Live Activities, and Schema domains (both need shell extension points that are not pure models).

## Tests

`intents.rs` tests added: `snippet_rows_keep_their_order_and_round_trip_through_a_result`,
`result_without_a_snippet_has_no_snippet_key`, `snippet_from_json_rejects_a_row_without_text`,
`double_parameter_takes_finite_numbers_only`, `currency_parameter_needs_an_amount_and_a_three_letter_code`,
`validate_checks_a_currency_parameter_by_its_declaration`. Not run (no builds in this wave).
