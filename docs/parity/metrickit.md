# MetricKit parity (team A4, GPU and Metal frame metrics)

A pure-Rust frame recorder for the GPU and frame-rate figures that GPUTimeMetric and MetalFrameRateMetric report.
It adds no crate, no unsafe code and no shell wiring.

## What was built

- **Frame recorder** (`springboard/core/src/frame_metrics.rs`, `Recorder`, `Frame`, `Summary`, `Error`).
  - `Recorder::new(capacity, refresh_ns)` keeps the last `capacity` presented frames (at least one) and the display
    refresh period they are measured against.
  - `record(gpu_ns, presented_ns)` adds a frame. Presentation times that go backwards are refused with
    `Error::OutOfOrder`. When the window is full the oldest frame leaves it.
  - `summary()` returns the frame count, mean and maximum GPU time per frame, the frame rate over the window (frames
    minus one, divided by the time between the first and last presentation) and the number of dropped frames. A
    frame interval longer than 1.5 refresh periods counts as one dropped frame.
  - `frames()` iterates the window, oldest first.
- Callers supply every figure in nanoseconds. Nothing reads a clock or a GPU counter.

## Official API names mirrored

- `GPUTimeMetric`: https://developer.apple.com/documentation/metrickit/gputimemetric
- `MetalFrameRateMetric`: https://developer.apple.com/documentation/metrickit/metalframeratemetric

## Matrix rows closed

| Framework | Feature (as in data/metrickit.json) | New status | BifrOSt path |
|---|---|---|---|
| MetricKit | GPU and Metal frame metrics (GPUTimeMetric, MetalFrameRateMetric) | partial | springboard/core/src/frame_metrics.rs |

The row stays partial. The GPU presenter in `springboard/src/gpu/wl.rs` does not yet supply GPU time or presentation
times to the recorder, and no app can reach it. Pixel luminance (PixelLuminanceMetric, AveragePixelLuminance) and
per-process energy impact are still gaps.

## Not done

- No SDK type or control socket method exposes the summary to apps.
- The presenter does not feed the recorder; per-layer measurement is missing.
- Nothing was built or run in this round (no cargo, by rule). The unit tests in `frame_metrics.rs` are written but not
  executed.
