# G1: frame clock and frame statistics (`wm-clock-stats`)

Owner: wave G1 agent `wm-clock-stats`. Contract: `docs/graphics-architecture.md`, "Window server",
`render_server/clock.rs` and "Performance budgets". Code: `springboard/wm/src/render_server/clock.rs`,
`springboard/wm/src/render_server/stats.rs`.

## What is implemented

Both modules are pure logic with no GPU, no sockets and no event loop, so they are host-tested.

### `FrameClock` (`clock.rs`)

- `new(refresh)`: a zero refresh falls back to 60 Hz (16 666 667 ns).
- `vblank(t)`: the phase anchor is the last vblank. A non-increasing timestamp is ignored. A delta within 10% of the
  refresh is an inlier and moves the refresh one eighth of the way toward it (exponential average). Any other delta,
  such as a missed vblank at about two periods or a long idle gap, moves the anchor only, so the refresh is not
  polluted by outliers.
- `frame_took(d)`: keeps the newest 120 compose+submit durations.
- `budget()`: 4 ms before any sample. Otherwise the nearest-rank 95th percentile of the recent frames plus 1 ms,
  clamped to 2..12 ms.
- `next_presentation(now)`: the anchor plus the smallest whole number of periods that lands strictly after `now`.
  With no vblank yet it is `now + refresh`.
- `start_at(now)`: `next_presentation(now) - budget()`, never earlier than `now`.

### Frame statistics (`stats.rs`)

- `Stats::push` keeps the newest `capacity` frames (a zero capacity keeps one).
- `cpu_percentile`, `gpu_percentile`: nearest-rank percentiles (rank `ceil(p/100 * n)`, clamped). Empty gives zero.
- `dropped_ratio`: the fraction of frames with `missed > 0`.
- `Stats::json_line(&FrameRecord)`: hand-written JSON, integer microseconds (see below).
- `append_frame(&FrameRecord)`: appends the line to `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` only when
  `BIFROST_WM_STATS=1`. `append_to(path, f)` does the write for a given path (used by the tests), `path_in` and
  `enabled_from` are the pure parts.

## Mapping to the contract

| Contract item | Where |
|---|---|
| Frame clock from vblank timestamps, budget learned from frame times, starting at 4 ms, clamped 2..12 ms | `FrameClock::vblank`, `frame_took`, `budget` |
| Compose scheduled at `next_vblank - budget` | `FrameClock::start_at` |
| Frame callbacks carry the predicted presentation time | `FrameClock::next_presentation` (the wiring sends it) |
| Stats written for the bench when `BIFROST_WM_STATS=1` | `stats::append_frame` |

## Frame timing (how `tools/bench.py` reads the file)

Format: one JSON object per line, appended in frame order. Keys and units:

```
{"seq":7,"cpu_us":1500,"gpu_us":2000,"latency_us":16000,"missed":1,"damaged_px":12345}
```

- `seq`: frame counter (unsigned, increases by one per composed frame).
- `cpu_us`, `gpu_us`, `latency_us`: integer microseconds. `latency_us` is submission to the vblank that showed it.
- `missed`: vblanks skipped since the previous frame (0 is smooth).
- `damaged_px`: pixels damaged in this frame.

Reading rules for `tools/bench.py --midget` (do not edit `bench.py` in this change; the bench owner adds the reader):

1. Copy the file from the device (`ssh midget`, `$XDG_RUNTIME_DIR` of the session user). Ignore a trailing partial
   line, since the server may be writing.
2. Compute percentiles with nearest rank over the frames of one run, for `cpu_us`, `gpu_us` and `latency_us`.
   Report p50, p95 and the maximum.
3. Dropped ratio is the count of frames with `missed > 0` divided by the frame count.
4. Compare p95 `cpu_us` against the 1000 us CPU budget and p95 `gpu_us` against the 4000 us GPU budget in
   `docs/graphics-architecture.md`.
5. Append the summary to `docs/perf-history.jsonl` in the same round as the bench run.

Start a run by setting `BIFROST_WM_STATS=1` in the server environment, and delete the file first so one run is
not mixed with another.

## Tests

`cargo check --offline -p bifrost-wm --tests` (run from `springboard/`) must pass. The tests in `clock.rs` and
`stats.rs` use hand-worked expected values: 16.125 ms after one 17 ms delta at 16 ms, the 2 ms and 3 ms budgets from the
p95 rank, nearest-rank percentiles of 1..10 ms (p50 5, p90 9, p95 10), the 0.4 dropped ratio of `[0,0,1,0,2]`, and the
exact JSON line.

## What is left (todo_left)

- Wire `FrameClock` into `drm.rs` (page-flip event sets `vblank`, the render loop calls `start_at`) and `nested.rs`
  (no vblank: use the nested frame callback time). Not done here: those files are owned by the output and compose
  agents.
- Feed `frame_took` and `FrameRecord` from `compose.rs`/`output.rs`. GPU time needs timestamp queries from
  `bifrost-gpu`, which the GPU layer provides.
- `wp_presentation` feedback and frame callback timestamps use `next_presentation`; the protocol wiring is in the
  presentation handler, not in this module.
- Refresh estimation has no mode seed check against EDID. If the first mode refresh is far from the panel (a 59.94 Hz
  panel reported as 60 Hz is fine; an unrelated value is not), the EWMA converges only within the 10% band.

## Verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

1. Build the fast profile and push the shell (`cargo build --offline --profile fast`, then the bifrost-wm target).
2. Set `BIFROST_WM_RENDERER=vulkan` and `BIFROST_WM_STATS=1` for the session.
3. Idle the desktop for 10 s, then move a window for 10 s, then read `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl`.
4. Expect `missed` to be 0 on almost every frame while idle, `cpu_us` p95 under 1000 and `gpu_us` p95 under 4000 on a
   typical desktop. The mode refresh on the panel is 60 Hz; `FrameClock::refresh` should settle within about 0.1% of
   the vblank spacing after a few hundred frames.
5. Record the summary with `tools/bench.py --midget` once the reader exists.
