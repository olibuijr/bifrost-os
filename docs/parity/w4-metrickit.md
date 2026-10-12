# Wave 4: MetricKit gap and OS rows (daily reports, call stacks, hitch ratio)

## What was built

Three pure std models in `springboard/sdk/src/`, each registered by one `pub mod` line in `springboard/sdk/src/lib.rs`.
None reads a clock, does I/O, or uses unsafe code. The SDK placement gives apps a module path to use; nothing in the
shell or in an app calls them yet.

- `springboard/sdk/src/daily_metric_report.rs` (`bifrost_app::daily_metric_report`). A `ReportSchedule` keeps a snapshot
  of the cumulative `bifrost_core::app_metrics::Totals`. The first poll takes the snapshot. A poll at least
  `REPORT_INTERVAL_MS` (24 h) later returns a `DailyPayload`: the field-by-field difference of the two snapshots over a
  `Window { begin_ms, end_ms }`, saturating at zero when a counter moved backwards. `PastPayloads` keeps the newest
  `MAX_PAST_PAYLOADS` (7) payloads. The 7-payload limit is a BifrOSt choice; the official reference only says past
  payloads last since the shared manager was allocated.
- `springboard/sdk/src/call_stack_tree.rs` (`bifrost_app::call_stack_tree`). A `CallStackTree` holds threads of
  `CallStackFrame` nodes. Sampled stacks (root first, `SampledFrame`) are merged so that shared prefixes share nodes,
  and every node counts its samples. Frames match on binary UUID plus offset into the text segment. Binary images sit
  once in a UUID-keyed table, first entry wins. `for_each_frame` walks depth first, `binary_name` resolves a frame, and
  `to_json` writes compact JSON. Counts saturate at `u32::MAX`.
- `springboard/sdk/src/hitch_ratio.rs` (`bifrost_app::hitch_ratio`). A `HitchMeter` records frame durations in
  microseconds against a budget. A frame over its budget adds the excess to hitch time. `ratio()` is hitch time over
  total frame time, 0.0 when nothing is recorded. `BUDGET_60HZ_US` is 16 667 µs.

## Official API names mirrored

- `MXMetricManager` (`add`, `remove`, `pastPayloads`): `https://developer.apple.com/documentation/metrickit/mxmetricmanager`
  The daily report subscription and the past-payloads list. The model has no subscriber registry and no delivery
  call; `ReportSchedule::poll` stands in for the system deciding a day has passed.
- `CallStackTree` (`callStackThreads`, `callStackPerThread`, `forEachFrame`, `binaryInfo`):
  `https://developer.apple.com/documentation/metrickit/callstacktree`
- `CallStackFrame` (`binaryUUID`, `address`, `offsetIntoBinaryTextSegment`, `sampleCount`, `subFrames`, `binaryName(from:)`):
  `https://developer.apple.com/documentation/metrickit/callstackframe`
- `CallStackTree.BinaryInfo` is mirrored as `BinaryInfo` with name and load address only. The official page lists more
  fields; those were not read in full.
- `HitchTimeMetric` (`ratio`, `totalHitchTime`, `totalAnimationTime`):
  `https://developer.apple.com/documentation/metrickit/hitchtimemetric`
  The official page says the ratio is normalised against the total animation duration and includes perceptual
  adjustments. `HitchMeter::ratio` divides hitch time by frame time and applies no perceptual weighting, so it is not the
  same number. This is recorded in the module doc.

The official `CallStackFrame`, `HitchTimeMetric` and `MXMetricManager` pages were read with `docc.py`. The
`MetricPayload` DocC path returned HTTP 404, so the daily payload shape is taken from the `pastPayloads` description
and the `MXMetricManager` page, not from a payload page.

## REA evidence

- `https://developer.apple.com/documentation/metrickit/callstacktree`: `ev_95590bda05b3a70a79843ec60bb4a197e8fbd9a24058f1f8d29541cbb288d1ee`
- `https://developer.apple.com/documentation/metrickit/hitchtimemetric`: `ev_e797e0c0a4fbcb31ec2d501a1bfab859a69a1fba0101dc3da833c7f6dc473298`
- `https://developer.apple.com/documentation/metrickit/mxmetricmanager`: `ev_50f9061fc408dfb7824ef34bba9be0944529a0f0d69c011c36a062267479a4e3`

## Matrix rows closed

Status is `partial` for each. The modules are pure models that no shell or app code calls, so none reaches `parity`.

| Framework | Feature | Status | BifrOSt path |
|---|---|---|---|
| MetricKit | Daily metric reports to the app (MXMetricManager, MetricReport) | partial | springboard/sdk/src/daily_metric_report.rs |
| MetricKit | Crash diagnostics (CrashDiagnostic, call stacks) | partial | springboard/sdk/src/call_stack_tree.rs |
| MetricKit | Animation hitch and slow-frame measurement (HitchTimeMetric, HitchTimeRatio) | partial | springboard/sdk/src/hitch_ratio.rs |

## Not done

- The shell does not feed `Totals` into `ReportSchedule`, and no delivery path hands a `DailyPayload` to an app.
- `springboard/src/bare/jank.rs` still classifies frames with its own 16.7 ms and 33 ms bands. It does not
  call `HitchMeter`.
- Hang duration histograms, memory and CPU exception diagnostics, and disk and GPU totals are still missing.
- Nothing has been compiled or run. The tests were checked by hand only, per the wave rules.
