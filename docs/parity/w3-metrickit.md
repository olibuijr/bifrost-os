# Wave 3: MetricKit gap rows (per-app usage metrics model)

## What was built

`springboard/core/src/app_metrics.rs` (`pub mod app_metrics;` in `springboard/core/src/lib.rs`), a pure std model
with no clock, no I/O and no unsafe. The caller supplies every timestamp in milliseconds from a fixed origin.
It keeps, per app name:

- Time in the foreground and in the background, from `set_phase` and `close`. Repeating the open phase keeps its start;
  a timestamp that moves backwards banks zero time.
- The first-draw time of each launch, from `begin_launch` and `first_draw`, in a 7-bucket histogram with bounds
  250, 500, 1000, 2000, 4000 and 8000 ms. Only the first draw after a launch counts.
- Foreground run endings counted by cause: `Normal`, `MemoryLimit`, `InvalidAccess`, `Abnormal`,
  `IllegalInstruction` and `Watchdog`, from `record_ending`.

Nothing in the shell or the SDK feeds the model yet. The Settings usage records in `springboard/src/shell/screentime/usage.rs`
stay the only per-app time store, and they are not changed. Audio and location time, the daily report
delivery and an SDK wrapper are not built.

## Official API names it mirrors

Documentation only, from the official pages (no memory was used):

- `TotalForegroundTimeMetric`, `TotalBackgroundTimeMetric` (time-in-use, foreground and background only).
  Audio and location time (`TotalBackgroundAudioTimeMetric`, `TotalBackgroundLocationTimeMetric`,
  `LocationActivityTimeMetric`, `CellularConditionTimeMetric`) are not mirrored.
- `TimeToFirstDrawMetric` (`histogram`). The optimized, resume and extended-launch variants are not mirrored.
- `ForegroundTerminationMetric` (`normalTerminationCount`, `memoryLimitTerminationCount`,
  `badAccessTerminationCount`, `abnormalTerminationCount`, `illegalInstructionTerminationCount`,
  `watchdogTerminationCount`). The background termination metric is not mirrored.

## REA evidence

- `ev_e6f48380cf012a620ae94bf29940767287b679bf5f5db903dc40a58df1dcc081`, MetricKit landing page (`rea-page.sh`)
- `ev_40ee1c7702c5b13387fed82a423a810b4648cde35dc0d030678e71d0657e61b9`, TotalForegroundTimeMetric
- `ev_dff661982ba1e1861f47245dc239bef5d0d82b3f8d0d4e1d364d589ca6ec9d1f`, TimeToFirstDrawMetric
- `ev_eb2d620ef74e0da905343263efce7a577466e70df065e202a1371e82b911d6f3`, ForegroundTerminationMetric

## Matrix rows closed

These rows move to `partial`. The model exists, and no app receives the data yet.

| framework slug | feature (as written in docs/parity-matrix/data/metrickit.json) | status | BifrOSt path |
|---|---|---|---|
| metrickit | Time-in-use metrics (foreground, background, audio, location time) | partial | springboard/core/src/app_metrics.rs |
| metrickit | Launch, resume and extended launch timing (TimeToFirstDrawMetric, ApplicationResumeTimeMetric, ExtendedLaunchMetric) | partial | springboard/core/src/app_metrics.rs |
| metrickit | Termination metrics (ForegroundTerminationMetric, BackgroundTerminationMetric) | partial | springboard/core/src/app_metrics.rs |

## Tests

Seven unit tests in `app_metrics.rs` (`#[cfg(test)] mod tests`), with expected values worked out by hand from the rule
each code path implements. They cover phase banking, a repeated phase, a backward clock, close with no open phase,
the first-draw timing and once-only rule, the bucket edges and ending counts. Not run: the owner rule for this
round forbids builds.

Owner name guard: `python3 tools/check-names.py` exits 0 with no hits.
