# 010 — Phase 8: measure and improve rendering

Status: planned. Baseline work depends on 002; code changes depend on 004. Parent: `001`.

## Problem (verified or noted)

- `docs/codebase-cleanup.md` records 31–35 ms draw times in a multi-window scene (Notes and App Store) with no matched baseline. This is a lead, not a measured regression or improvement.
- Touchpoints: `bare/jank.rs`, `bare/gpu_comp.rs`, `bare/scanout.rs`, `bare/wayland.rs`, shell draw and composition, SDK damage and buffer handling, and `springboard/docs/zero-copy-scanout.md`.
- Review the existing scanout paths before adding another renderer.

## Steps

1. Instrument the latency path with consistent clocks and units: input receipt, dispatch, app work, draw, buffer submission, composition, and actual presentation. Do not treat submission as display.
2. Define reproducible scenes (from 002): multiple visible windows, scrolling, resize, and idle. Record hardware, resolution, build profile, and trace overhead for each run.
3. Record p50, p95, and p99 frame time, missed frames, idle wakeups and CPU, process-tree memory, copied bytes, and trace overhead.
4. Before/after on the same scenes. Remove the most expensive redundant drawing, copying, or scheduling work first.
5. Preserve retained damage and the software fallback.
6. Introduce GPU-backed or shared surfaces only when the measured bottleneck supports it. Require explicit synchronization, format negotiation, resize cleanup, device-loss handling, and resource bounds.

## Acceptance

- Matched before and after runs improve the identified bottleneck.
- No stale pixels, damage errors, latency regressions, or uncontrolled memory growth.
- Results recorded in `springboard/docs/platform-maturity.md` with sample counts.
- VM and physical-hardware performance are reported as separate claims. Emulator numbers are not hardware numbers.
