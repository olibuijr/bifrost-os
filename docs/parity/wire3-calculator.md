# Wire 3: Calculator (paper tape and the measurement and number formatting models)

Status: wiring wave 3 (0.5.39 branch `wire3/calculator`), app `springboard/apps/calculator-desktop` 1.5.0. Not built on
the wiring side; host tests written and not run. Builds and the Midget run are the lead's first checks.

## What is now wired

- **Paper tape.** The calculation history sidebar has a Copy Tape button on the left of its bottom row (Clear History
  keeps the right half). Copy Tape writes the whole history, oldest first, to the shared clipboard as plain text:
  a title line, then each expression with `= result` on the line below, right-aligned to 32 columns. Results use the
  calculator's language (decimal comma when Icelandic is chosen). Copy Tape is disabled while the history is empty.
- **Model.** `src/tape.rs` (`render`, `WIDTH`, `TITLE`). Pure std, unit tests in the same file.
- **Accessibility.** The button is a named `Button` control ("Copy Tape") in `controls()`, enabled only with entries.
- **Measurement and number formatting.** Unchanged from wave 1 (convert.rs): length, mass and temperature use the SDK
  `Measurement` model; results use the SDK `NumberFormatter`. The wave-1 gaps still apply (see below).

## Where the code is

- `src/tape.rs`: the tape model and its tests.
- `src/calc.rs`: `ID_COPY_TAPE`, `Target::CopyTape`, `copy_tape()`, the split bottom row in `controls()`, `hit()`,
  `draw_history()`, and the test `the_bottom_row_copies_the_tape_on_the_left_and_clears_on_the_right`.
- `src/main.rs`: `mod tape;`.
- `Cargo.toml`, `manifest.toml`: 1.5.0. `CHANGELOG.md`: 1.5.0 entry.

## How a user reaches it

Open Calculator, make a few calculations, and open the history sidebar (the History button in the toolbar). Tap the
left half of the bottom row, "Copy Tape". Paste into any text field (for example Notes) to read the tape.

## How to verify on the test PC (Midget)

Use Midget, not a VM.

1. `bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`.
2. `bifrost pc midget open-app is.olibuijr.calculator.desktop`.
3. Type `2+3=`, then `10/4=`. Open History (toolbar).
4. `bifrost pc midget tap-window Calculator X Y` on the left half of the bottom row (window-local pixels from the
   screenshot; use `tap-window`, not `tap`).
5. Open Notes, create a note, and paste (Edit > Paste). The text should read:
   `Calculator Tape`, `2+3`, then `= 5` right-aligned, `10/4`, then `= 2.5` right-aligned.
6. Set Decimal to Icelandic, copy again, and check the result line reads `= 2,5`.
7. `bifrost pc midget screen /tmp/tape.png` in Light and Dark (`bifrost pc midget appearance light|dark`); check with
   `bifrost-hig`.

Host tests (for the lead): `cargo test -p bifrost-calculator` in `springboard/` covers `tape::tests` and
`calc::tests::the_bottom_row_copies_the_tape_on_the_left_and_clears_on_the_right`.

## REA evidence (official documentation, rendered pages)

- `ev_510adbd6df3969d5fa6e703ef25c038144c3c715d7459a479781d5ce7c966d3d`: MeasurementFormatter
  (`https://developer.apple.com/documentation/foundation/measurementformatter`): `string(from:)`, `unitStyle`,
  `numberFormatter`. Checked with `python3 -I tools/parity/docc.py foundation/measurementformatter`.
- `ev_64567bf81202561f38f9299ccfb9ad5697dcf6cef8b77d09826adf178b8126a3`: NumberFormatter
  (`https://developer.apple.com/documentation/foundation/numberformatter`), rendered page.
- Wave 1 ids (Measurement, NumberFormatter, UnitSpeed, UnitVolume) are in `docs/parity/wire-app-calculator.md`.

The paper tape itself has no Apple API counterpart in the reference documentation; it is a BifrOSt Calculator feature
built on the existing history model.

## Remaining gaps

- **Speed and volume are not in the SDK model.** `bifrost_app::locale::Unit` has length, mass and temperature only.
  The fix belongs in `springboard/sdk/src/locale.rs`, outside this app (wave 1 gap).
- **MeasurementFormatter is not used for the unit chips.** The chips show plain unit names; the SDK `measurement()`
  formatter could show localized unit symbols. Not wired in this wave.
- **The tape is copied, not printed or shown in a view.** There is no tape window or print sheet; the tape exists as
  text on the clipboard only. Convert results are not added to the tape.
- **Not kept across a restart.** Convert state is in memory only (wave 1). The history itself persists as before.
- **Tape width is fixed at 32 columns.** A long expression is kept whole on its own line and is not wrapped.
- **Not built or run.** The lead's build and the Midget run are the first checks.

## Matrix rows

| Slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| foundation-fundamentals | Units and Measurement | `springboard/apps/calculator-desktop/src/convert.rs` (Measurement via `bifrost_app::locale`), Convert mode in `src/calc.rs` |
| foundation-fundamentals | Data Formatting | `springboard/apps/calculator-desktop/src/convert.rs` (`format_result`, NumberFormatter); paper tape `src/tape.rs` |

Matrix data is not edited here; the lead updates the rows after the build.
