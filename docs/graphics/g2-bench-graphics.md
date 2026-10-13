# G2: graphics benchmark (`bench-graphics`)

Owner: wave G2 agent `bench-graphics`. Contract: `docs/graphics-architecture.md`, "Performance budgets" and
"Switch-over"; row 19 of `docs/graphics-waves.md`. Code: `tools/graphics_bench.py` (helper), `tools/test_graphics_bench.py`
(tests), `tools/bench.py` (the `--graphics` hook). Python only; no Rust was changed in this round.

## What is implemented

`tools/graphics_bench.py` reads the window server's frame file and gates it against the budgets:

- Frame file: `$XDG_RUNTIME_DIR/bifrost-wm-frames.jsonl` (`/run/user/1000/...` on Midget), written by
  `springboard/wm/src/render_server/stats.rs` when `BIFROST_WM_STATS=1`. The reader skips blank lines, malformed lines
  and a trailing partial line (the server may be writing).
- Percentiles: nearest-rank, the same definition as `stats.rs` (rank `ceil(p/100 * n)`, clamped). p50, p95 and p99 are
  reported for compose CPU, GPU time and present latency.
- Dropped-frame ratio: the fraction of busy frames (`damaged_px > 0`) with `missed > 0`. On an idle desktop with no busy
  frames the ratio covers every frame.
- Input-to-photon: `inject_click(x, y)` brackets `tools/midget.py tap X Y` with the device's monotonic clock
  (`time.monotonic_ns()` read over ssh before and after). The photon time of a click is the first frame whose `t_us` is at
  or after the click's end bracket. The bracket includes the ssh round trips, so the latency is an upper bound.
- Gate (exit 3 on a miss, listed on stdout; a missing measure is also a miss):

| Key | Budget | Source |
|---|---|---|
| `gfx_cpu_p95_ms` | <= 1.0 ms | "under 1 ms CPU" (window server frame) |
| `gfx_gpu_p95_ms` | <= 4.0 ms | "under 4 ms GPU" (window server frame) |
| `gfx_dropped_ratio` | <= 0.01 | "60 fps with no dropped frames while the app is busy" (1 % allowance; the contract says zero, so tighten once measured) |
| `gfx_photon_p95_ms` | < 25 ms | "Input to photon (click, key) under 25 ms" |

Reported without a gate: `gfx_*_p50_ms`, `gfx_*_p99_ms`, `gfx_latency_*` (present latency), `frames`,
`gfx_busy_frames`, `gfx_photon_clicks`.

- Results are appended to `docs/perf-history.jsonl` (same entry shape as `bench.py`) unless `--no-record`.

Commands:

```
python3 tools/graphics_bench.py --file FILE --no-record      # analyse a local copy of the frame file
python3 tools/graphics_bench.py --midget [--click X Y]       # read Midget, optionally inject one click first
python3 tools/bench.py --midget --graphics [--click X Y] --no-record   # the suite next to the footprint numbers
python3 -I -m unittest discover -s tools -p 'test_graphics_bench.py'   # 24 tests, pure logic
```

`tools/bench.py` gets a `--graphics` flag (with `--midget`). It merges the graphics results into `res` and adds the
graphics breaches to the exit-3 list. Without `--graphics` the bench is unchanged.

## Mapping to the contract

| Contract item | Where |
|---|---|
| Frame stats for the bench (`bifrost-wm-frames.jsonl`) | `parse_frames`, `summarize` |
| Compose CPU and GPU percentiles, dropped frames, present latency | `summarize`, `dropped_ratio`, `busy_frames` |
| Input-to-photon: uinput click to the first presented frame showing it | `inject_click`, `photon_latencies_ms` |
| Budget gate, exit non-zero on a miss | `BUDGETS`, `gate`, `main` (exit 3) |
| Results in `docs/perf-history.jsonl` | `record` |
| Midget only, `bench --midget` reports the frame budgets | `tools/bench.py --midget --graphics` |

## Limits and what is not done

- The frame records have no timestamp, so input-to-photon needs a `t_us` key (see contract change below). Until the
  window server writes it, `gfx_photon_*` is reported as not measured and the gate fails. That is deliberate.
- The "test surface that changes colour" is not built. `inject_click` only injects; the test app must draw a colour
  change on click (a direct-path app, G2 row 14/11). The screencopy check that the frame shows that colour is not
  written either: `tools/midget.py` has no screencopy reader yet.
- Animation smoothness during a window open and a Dock magnification sweep is covered only by the dropped-frame ratio
  over busy frames. The sweep script (driving the Dock with `tools/midget.py`) is not written.
- App redraw time (the contract's "re-record and draw one layer, under 2 ms") is not measured.
- Budget values are the contract's numbers. The 1 % dropped-frame allowance is an interpretation of "no dropped frames";
  it should be tightened when Midget numbers exist.
- Not run on hardware in this round: Midget was unreachable from the build host (`ssh: No route to host`, 192.168.20.140),
  so no numbers are recorded in `docs/perf-history.jsonl`.

## Contract changes

- The frame record gets an optional `"t_us"` key: CLOCK_MONOTONIC microseconds when the frame was shown (the
  `wm-stats` owner adds it in `stats.rs` `FrameRecord`, with the vblank time). The reader already accepts it.

## How to verify on Midget (Intel HD 530, Mesa ANV, Vulkan 1.4)

The session (history: ran labwc before its removal) runs `bifrost-wm`; formerly unless `/etc/bifrost/compositor` says `bifrost-wm`. Then:

1. Test the helper on the host: `python3 -I -m unittest discover -s tools -p 'test_graphics_bench.py'` (24 tests, all pass).
2. Confirm Midget is reachable: `python3 tools/midget.py state`, then `ssh midget`.
3. On Midget, set `/etc/bifrost/compositor` to `bifrost-wm` and `BIFROST_WM_RENDERER=vulkan`, `BIFROST_WM_STATS=1` in
   the session environment, then restart the session (`bifrost pc midget update --restart` or a reboot).
4. Let the desktop idle for 30 s, then `python3 tools/graphics_bench.py --midget --no-record` for the idle numbers.
5. Open and close a window and sweep the Dock magnification, then run `python3 tools/bench.py --midget --graphics --no-record`
   (add `--click X Y` once the test surface exists). Exit 3 lists each budget miss.
6. Recheck that the labwc path still works (set `/etc/bifrost/compositor` back to labwc and run step 5 without
   `BIFROST_WM_STATS`; the file then does not exist, which the helper reports with exit 2).

## todo_left

- Add `t_us` to the frame record (`springboard/wm/src/render_server/stats.rs`).
- Build the colour-change test surface and the screencopy check for the photon measure.
- Write the Dock magnification sweep and window-open driver for the animation measure.
- Measure app redraw time (one layer re-record) and add `gfx_redraw_p95_ms` with a budget of 2 ms.
- Record the first Midget run in `docs/perf-history.jsonl` once Midget is reachable.
