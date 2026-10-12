# Wire 5: Calculator (statistics on the paper tape)

Status: wiring wave 5 (branch `wire5/calculator`, from `main` at 0.5.42), app `springboard/apps/calculator-desktop`
1.6.0. Not built on the wiring side; host tests written and not run. Builds and the Midget run are the lead's first
checks.

## What is now wired

- **Statistics block on the paper tape.** Copy Tape (wave 3) now ends with a blank line and a `Statistics` block
  after the calculations: `Count`, `Sum`, `Mean`, `Std Dev` (two or more calculations only), `Min`, `Max`. Each label
  is on the left and its value right-aligned to the tape's 32 columns. The block is left out when the history is empty.
- **Model.** The values come from the SDK's `NumericSummary` (`bifrost_app::controls::column_summary`, the Tabular
  Data statistics model in `springboard/ui/src/column_summary.rs`). The sum is added in the tape model itself because
  `NumericSummary` has no sum field. NaN values are treated as missing by the model.
- **Number formatting.** The statistics use the calculator's `format::display`, so they follow the Decimal setting
  (decimal comma when Icelandic is chosen), the same as the results above them.
- **Unchanged.** Convert mode, the history sidebar, Copy Tape's place in the bottom row and the Copy Tape control are
  as in wave 3 and 1.4.0.

## Where the code is

- `src/tape.rs`: `statistics_lines`, `row`, `STATISTICS`, the `NumericSummary` import, and the tests
  `statistics_of_three_values_are_worked_out_by_hand`, `the_statistics_block_follows_the_entries_after_a_blank_line`,
  `icelandic_results_and_statistics_use_a_decimal_comma`. `render` now calls `statistics_lines`.
- `src/calc.rs`: unchanged in this wave (`copy_tape` already passes the whole history to `tape::render`).
- `Cargo.toml`, `manifest.toml`: 1.6.0. `CHANGELOG.md`: 1.6.0 entry.

## How a user reaches it

Open Calculator, make a few calculations, and open the History sidebar. Tap the left half of the bottom row, Copy
Tape. Paste into Notes (or any text field). The tape ends with the Statistics block.

## How to verify on the test PC (Midget)

Use Midget, not a VM.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. `bifrost pc midget open-app is.olibuijr.calculator.desktop`.
3. Type `1+0=`, `2+0=`, `3+0=`. Open History.
4. `bifrost pc midget tap-window Calculator X Y` on the left half of the bottom row (window-local pixels from the
   screenshot; use `tap-window`, not `tap`).
5. Paste into Notes. After the three calculations the tape should end with:
   `Statistics`, then `Count` with `3`, `Sum` with `6`, `Mean` with `2`, `Std Dev` with `1`, `Min` with `1`, `Max`
   with `3`, each value right-aligned to the 32nd column.
6. Set Decimal to Icelandic, copy again, and check the Mean line reads `2`, and a result such as `2.5` reads `2,5`.
7. Check the History panel look in Light and Dark (`bifrost pc midget appearance light|dark`,
   `bifrost pc midget screen /tmp/tape.png`); check with `bifrost-hig`. The panel itself is unchanged.

Host tests (for the lead): `cargo test -p bifrost-calculator` in `springboard/` covers `tape::tests`.

## REA evidence (official documentation)

- Official reference read with `python3 -I tools/parity/docc.py tabulardata/numericsummary`: NumericSummary
  (`https://developer.apple.com/documentation/tabulardata/numericsummary`) lists `someCount`, `noneCount`, `mean`,
  `standardDeviation`, `min`, `max`, `median`, `firstQuartile` and `thirdQuartile`. The page does not state the
  standard deviation divisor or the quartile method; the model's choices are documented in `column_summary.rs`.
- No REA rendered-page evidence id was recorded in this wave (the rendered page was not captured). Evidence ids from
  earlier waves for the formatter and measurement pages are in `docs/parity/wire-app-calculator.md` and
  `docs/parity/wire3-calculator.md`.

## Remaining gaps

- **No Statistics mode or panel.** The statistics are only on the copied tape. There is no visible Statistics view,
  no Programmer mode and no bitwise or base-conversion keys.
- **No vector or matrix arithmetic in the app.** `vector_ops` (vDSP-style, f32) and `linalg` are not used by the
  calculator; the tape needs only the summary model.
- **Convert results are not on the tape.** Convert mode (wave 1) still keeps its results out of the history.
- **Speed and volume are not in the SDK model** (`bifrost_app::locale::Unit` has length, mass and temperature only;
  fix in `springboard/sdk/src/locale.rs`, outside this app).
- **Unit symbols are plain text.** The SDK `MeasurementFormatter` is not used for the unit chips.
- **Quartiles and median are not on the tape.** The model computes them; the tape shows only the rows listed above.
- **Not built or run.** The lead's build and the Midget run are the first checks.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| tabular-data | Statistical summaries | `springboard/apps/calculator-desktop/src/tape.rs` (`statistics_lines`, via `bifrost_app::controls::column_summary::NumericSummary`) |
| foundation-fundamentals | Data Formatting | `springboard/apps/calculator-desktop/src/tape.rs` (`render`, `statistics_lines` use `format::localize` and `format::display`) |

Matrix data is not edited here; the lead updates the rows after the build.
