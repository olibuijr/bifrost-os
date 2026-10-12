# Wire: Calculator (unit conversion and number formatting)

Status: wired in `springboard/apps/calculator-desktop` 1.4.0 (not built on the lead's side yet; host tests written,
not run). Matrix slug `foundation-fundamentals`, features "Units and Measurement" and "Data Formatting"
(`docs/parity-matrix/data/foundation-fundamentals.json`, not edited here).

## What is now wired

- **Convert mode.** A third segment, Convert, sits in the toolbar group with Basic and Scientific. It replaces the
  scientific keypad with a converter that uses the same keypad. The Calculator's own state is untouched while converting.
- **Categories.** Length, Mass, Temperature, Speed and Volume, as a segmented row under the toolbar. Choosing a
  category sets the From and To units to its first two units.
- **Units.** Two chips, "From: <unit>" and "To: <unit>". Tapping a chip moves to the next unit in its category. The
  Swap control exchanges them.
- **Entry and result.** Digits, the decimal key, +/−, Delete and Clear edit the amount in the From unit. The large
  display shows the amount converted to the To unit; the line above shows the entry and the From symbol.
- **Number formatting.** The result uses the SDK's `NumberFormatter` (`bifrost_app::locale`): grouping and decimal
  separators follow the app's language (the Decimal button toggles English and Icelandic), at most six fraction
  digits. Very large or very small values keep the Calculator's scientific notation.
- **Edit menu and accessibility.** Copy takes the converted value as plain digits; Paste sets the entry. Every
  category, unit chip and Swap is published as a named control.
- **Units and formulas.**
  - Length (m, km, mi, ft), mass (g, kg, lb) and temperature (°C, °F, K) are converted by the SDK's `Measurement`
    model (`Measurement::converted`, `Unit`) through `Kind::Std`.
  - Speed (m/s, km/h, mph, kn) and volume (mL, L, US fl oz, US gal) use fixed factors to a base unit (`Kind::Scale`).

## Where the code is

- `src/convert.rs`: the pure model. `Category`, `UnitDef`, `convert`, `Converter` (entry, units, swap, cycling),
  `format_result`. Unit tests are in the same file.
- `src/calc.rs`: the wiring. `BAR_CONVERT`, the `Target::{ShowConvert, LeaveConvert, Category, From, Swap, To}`
  variants, `convert_hit`, `draw_convert`, `category_rect`, `unit_rects`, routing in `press_basic`, `copy_text`,
  `paste_text`, `recall`, `display_text`, `expression_text`, `controls`, and `describe` (`calculator_convert=`).
- `src/calc/convert_tests.rs`: touch-driven test of Convert mode at 1200 x 1920 (open, type, pick units, swap,
  category, copy, draw, leave).
- `src/main.rs`: `mod convert;`.

## How a user reaches it

Open Calculator, then choose Convert in the toolbar (between Scientific and the Decimal button). Choose a category
at the top, then tap a unit chip to change it, type the amount on the keypad, and read the result on the display.
Tap Basic or Scientific, or Convert again, to return to the calculator.

## How to verify on the test PC (Midget)

This branch is not built here. Use Midget, not a VM.

1. Build and publish as the release flow says: `bifrost pc publish`, then `bifrost pc midget ota` until
   `phase=accepted`.
2. `bifrost pc midget open-app is.olibuijr.calculator.desktop`, then `bifrost pc midget state` to read the Calculator
   window.
3. `bifrost pc midget tap-window Calculator X Y` on the Convert segment (window-local pixels from the screenshot). Use
   `tap-window`, not `tap`.
4. Type `1`, `0` on the keypad, then tap the To chip twice. The display should read `32.808399` (10 m in feet).
5. Tap Swap; the From chip reads "Feet". Tap the Temperature segment; the chips read Celsius and Fahrenheit.
6. Set Decimal to Icelandic and check the result shows a decimal comma.
7. `bifrost pc midget screen /tmp/convert.png` for the Light and Dark look (`bifrost pc midget appearance light|dark`),
   then check it with `bifrost-hig`.

Host tests (for the lead, once the build runs): `cargo test -p bifrost-calculator` in `springboard/` covers
`convert::tests` and `calc::convert_tests`.

## REA evidence (official documentation, rendered pages)

- `ev_d709d41ded9c4ad0881d4abaf544d032b7c40a17f724b7ac48c47fc3178200f6`: Measurement
  (`https://developer.apple.com/documentation/foundation/measurement`): `init(value:unit:)`, `convert(to:)`,
  `converted(to:)`.
- `ev_72a40e78014ab051e919924e95f201e4a472152805661e73266eb1f47c992754`: NumberFormatter
  (`https://developer.apple.com/documentation/foundation/numberformatter`).
- `ev_f119f4cadb0940d5c26a3af5a5b3329d8730ea1da92655161d3801401603e04a`: UnitSpeed
  (`https://developer.apple.com/documentation/foundation/unitspeed`).
- `ev_de76917de7221ff92558deb3015da71af250597dcb6cc892855c66093aa4d812`: UnitVolume
  (`https://developer.apple.com/documentation/foundation/unitvolume`).

`python3 -I tools/parity/docc.py foundation/measurement`, `foundation/unitlength`, `foundation/unitspeed`,
`foundation/unitvolume` and `foundation/measurementformatter` were checked for the same surface.

## Remaining gaps

- **Speed and volume are not in the SDK model.** `bifrost_app::locale::Unit` has length, mass and temperature only.
  The app's scale table is an adapter. The fix belongs in `springboard/sdk/src/locale.rs` (add the dimensions to
  `Unit`), outside this folder, after which the two categories should move onto `Kind::Std`.
- **No user-defined units, no unit popup.** Units cycle on tap. Apple's `Unit` subclasses and a picker are not built.
- **Not kept across a restart.** The converter's category, units and entry are in memory only.
- **No App Intent for conversion.** Only the existing Evaluate intent exists.
- **Layout on short windows.** The two rows sit at 60 to 120 pt. On the 1200 x 780 smoke size (about 499 pt tall)
  the entry line may sit under the unit row; check it with `bifrost pc smoke calculator-desktop --sizes 1200x780 --scale 1.5625`.
- **Basic and Scientific leave Convert** rather than switching the keypad in one tap.
- **Clear key label** keeps the calculator's label ("AC") while converting.
- **Not built or run.** The branch was written without building; the lead's build and the Midget run are the first checks.
