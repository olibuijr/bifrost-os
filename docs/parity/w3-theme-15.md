# Wave 3, theme-15 (Video assets): weather pressure trend and historical statistics

Theme-15 lists rows from six frameworks (PhotoKit, DiskImageKit, StoreKit, Address Book, PDFKit, WeatherKit). Most of
those rows need hardware, a store service or a file format with no pure-Rust counterpart, so this round closes the two
WeatherKit rows that a pure model can honestly cover. The others are left open (see "Not closed").

## What was built

- `springboard/ui/src/pressure_trend.rs` (`pub mod pressure_trend;` in `springboard/ui/src/lib.rs`): `PressureTrend`
  (`Falling`, `Rising`, `Steady`) with the official description wording, and `classify`, which takes Unix-second
  readings in hectopascal, uses only finite readings inside a window ending at `now`, and compares the earliest with
  the latest reading. A change inside the steady band is `Steady`. Readings covering under half the window give `None`.
  The window (3 h) and the band (1 hPa) are BifrOSt defaults; the official documentation does not publish thresholds.
- `springboard/ui/src/weather_stats.rs` (`pub mod weather_stats;` in `springboard/ui/src/lib.rs`): `percentile`
  (linear interpolation between ranks), `Summary` and `summarize` (count, min, max, mean, median), `trend_per_step`
  (least-squares slope against position), and `Deviation` with `compare`, which places a current value above, below or
  inside the interquartile range (25th to 75th percentile) of a baseline. Non-finite values are ignored throughout.

Both modules are pure: they take their data as arguments and read no clock, no network and no file. Each has a
`#[cfg(test)] mod tests` with hand-worked expected values (for example, the slope of 1, 3, 5, 7 is 2.0, and the
baseline 1 to 9 has quartiles 3 and 7).

## Official API names mirrored

- WeatherKit `PressureTrend` (cases `falling`, `rising`, `steady`; `description`): https://developer.apple.com/documentation/weatherkit/pressuretrend
- WeatherKit statistics and comparison types, mirrored as the summary, percentile, trend and comparison functions:
  `DailyWeatherStatistics`, `Percentiles`, `Trend`, `HistoricalComparisons`, `Deviation`
  (https://developer.apple.com/documentation/weatherkit; DocC for `dailyweatherstatistics` checked with `tools/parity/docc.py`).

The statistics types above are not reproduced one for one. Apple's types carry Apple-service data (baseline start year,
per-day records, metadata). BifrOSt only models the arithmetic over values the caller already holds.

## REA evidence

- `ev_e9109bbdd53a6ad3ea46819da3389e2f06c53b46707a7ddf3af2106079684029`: rendered WeatherKit landing page, which lists
  `Pressure Trend`, `Daily Weather Statistics`, `Percentiles`, `Trend`, `Historical Comparisons` and `Deviation`.
- `ev_f708cffab4d15f37f8f7a6fb418029067118a80c1ffdbc9663201f225406c66e`: rendered `PressureTrend` page (cases and
  description wording).

## Matrix rows closed

| framework slug | feature (as in data file) | new status | BifrOSt path |
|---|---|---|---|
| weatherkit | Pressure trend | partial | springboard/ui/src/pressure_trend.rs |
| weatherkit | Historical statistics and comparisons | partial | springboard/ui/src/weather_stats.rs |

Both are `partial`, not `parity`: the models are not yet fed by any weather source (the forecast request still does not
ask for pressure, and no historical data is fetched), and they are not wired into the Weather app or the SDK.

## Not closed in this theme

- PhotoKit, DiskImageKit, StoreKit, Address Book, PDFKit rows: need a photo library, a disk image format, a store
  service, a contacts image store or a PDF write path that BifrOSt does not have. A pure model would not be honest for
  any of them, and PDFKit write options need a PDF rewriter, which is a larger change than this theme.
- WeatherKit "Precipitation" and "Minute forecast" already have wave 2 models; they are not changed here.

## Verification

Not built or run in this round (the wave brief forbids builds). The code was checked by hand against the test values,
and `python3 tools/check-names.py` reports no hits for the two new files.
