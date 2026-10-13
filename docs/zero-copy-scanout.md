# Zero-copy scanout and GPU compositing plan (OLI-695)

## State (shell 0.7.25)
Zero-copy scanout is implemented for the bare (tablet, no Wayland) path and is ON when the `gpu_scanout` pref is unset; `0` turns it off. The Wayland PC session never uses it (`!wayland` in `bin/shell.rs`): `bifrost-wm` owns scanout there. Pieces:
- `bare/scanout.rs`: kernel step (dumb buffer exported as dma-buf), pure decision table (`decide`), `FrameSwitch`.
- `gpu/scanout.rs`: runtime probe (KMS + EGL import for XB24/XR24, both methods).
- `gpu::Gpu::enable_scanout` / `present_scanout` / `disable_scanout`: render straight into 3 scanout buffers, page-flip the finished one.
- `bin/shell.rs`: enabled at start when the pref `gpu_scanout` is unset or `1` (Settings > Developer > Zero-Copy GPU Output, restart); flip failure or GPU loss drops back to the copy path and frees the buffers.

## Host benchmark (`cargo run --release --example scanout_bench`)
The CPU copy-out that scanout removes, 1920x1200, ms per frame (x86 host, 4 threads available):

| case | swap R/B 1t | 4t | plain copy 1t | 4t |
|---|---|---|---|---|
| full frame | 0.72 | 0.45 | 0.61 | 0.36 |
| half frame (swipe) | 0.23 | 0.12 | 0.16 | 0.09 |
| dock + status bar | 0.07 | 0.04 | 0.05 | 0.03 |
| one tile | 0.01 | 0.01 | 0.01 | 0.01 |

Host memory bandwidth is far above a write-combined dumb buffer's, so these are lower bounds; the on-device figures (9-12 ms large-damage frames) are dominated by that uncached write. What the numbers do show: the copy cost scales with damage, so small frames gain little and full-frame/large-damage frames (rotation, app open, switcher, wallpaper change) gain most; the R/B swap costs ~20-40 % over a plain copy, which XB24 scanout avoids entirely.

## Plan
1. Done: probe, buffers, render-to-scanout, flip, fallback, Developer toggle (flag).
2. On-device trial (user present, per HANDOFF: never deploy while they are on the machine): enable the toggle, restart, check `bifrost logs` for "zero-copy scanout on", compare frame times for rotation/app open/switcher against the copy path using the perf HUD; look for tearing, stale regions (damage + 3 buffers) and suspend/resume (buffers and KMS state across sleep).
3. (default flipped to on when the pref is unset; the criteria stay the check) Go/no-go for default-on: no flip failures over a day, large-damage frames at least 3 ms faster, no visual artifacts, suspend/resume clean. The toggle stays as the escape hatch.
4. Later: GPU blur for materials and Settings push/pop (keeps frames on the GPU instead of falling back to CPU composition), GSP 2D blitter (`/dev/gsp`) for the remaining CPU paths.

Owner of the device trial: QA Engineer (Final QA), needs the user's go-ahead.
