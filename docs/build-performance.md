# BifrOSt build and iteration performance

Measured on Titan, 2026-10-07. First desktop update: 0.3.18. These numbers are history: the shell is 0.7.x now, the
workspace is larger and `bifrost pc publish` builds more than the shell (see "Applied"). Re-measure before quoting them.
Current device budgets and measurements are in [snow-leopard.md](snow-leopard.md), `budgets.toml` and `perf-history.jsonl`.

| Work | Time |
| --- | --- |
| Optimized fast build after the display change/version bump | 36.92 s |
| Warm no-change build | 3.36 s |
| Rebuild after a comment edit in Settings page source | 3.86 s |
| Package and publish a previously built binary | 2 s |

The comment-edit measurement validates the incremental frontend/link path; a
behavioral change can invalidate more optimized code and take longer. Full LTO
was interrupted after several minutes, so there is no completed release timing
or measured full-LTO/fast runtime performance comparison.

## Applied

- `cargo build --profile fast` and `bifrost pc publish` use optimized `fast`, not full LTO (`bifrost pc publish --release`
  opts in). Keep that default. It uses opt-level 3,
  32 codegen units, incremental compilation and the existing lld linker.
- Package only the system binaries, never the apps. `tools/pc-publish.py` builds three cargo invocations: the shell, `bootweave`,
  `bifrost-portal` and `bifrost-privd` (with `desktop-wayland`); `bifrost-mcp`, `bifrostd`, `bifrost-audiod` and the audio
  pieces; and `bifrost-wm`, the chrome clients and `bifrost-log`. It also builds the EFI loader from source every time
  (`tools/build-efi-boot.sh release`, checked by `tools/verify-efi-loader.sh`; the loader is outside the workspace) and syncs
  and checks the package mirror (`tools/pc_mirror.py`) before the build, which costs download time on the first run after
  an Arch update.
- Strip the downloadable copy, retaining symbols in the local binary. The first
  update is 4,640,999 bytes (approximately 4.4 MiB); assets are already installed.
- Publish a validated artifact with `bifrost pc publish --skip-build` to avoid even
  another Cargo no-op. Normal publishing still builds before packaging.
- Build/publish prints separate phase times, and builds generate Cargo HTML
  timing reports under `springboard/target/cargo-timings/`.
- Test desktop code with relevant filters and its desktop preview (`bifrost pc smoke APP`).
- `[profile.fast]` (in `springboard/Cargo.toml`) inherits `release` with `lto = false`, 32 codegen units, incremental, no
  strip; `springboard/.cargo/config.toml` links x86_64 with lld.
- Agents never build. The lead's one integration build is `BIFROST_LEAD_BUILD=1 tools/lead-check.sh`; a repo-local
  `rust-analyzer.toml` with `checkOnSave = false` keeps the editor from re-checking the workspace on every change
  ([bifrost-pc-workflow.md](bifrost-pc-workflow.md), "Agent waves").
- Build and test with `--all-targets` when checking: `cargo test` builds examples that `cargo check --tests` skips.

## Iteration discipline

Use `cargo check --lib` while editing, then targeted `cargo test --lib FILTER`
with the appropriate platform. Render `desktop_settings_preview` for this pane
instead of booting/installing an ISO. Run the full workspace acceptance once
before shipping the batch. Build after the last code/version edit; do not change
source under a running compilation or launch duplicate builds for one target.
Keep compiler flags, toolchain, target directory and profile stable to preserve
Cargo and incremental caches. Do not run `cargo clean` as routine maintenance.

## Further opportunities, not yet benchmarked or enabled

- The debug target is 160 GiB. Use limited or line-table debug information for
  routine tests/previews, reserving full local-variable information for a
  dedicated debugging profile. This causes a one-time cache rebuild.
- Bound simultaneous compilation/linking for full-workspace tests: Titan has
  16 logical CPUs, but concurrent linkers can each create many workers. Measure
  `-j4`, `-j8`, and linker worker limits before selecting a default.
- Separate the large browser/media dependency graph from basic shell/UI tests
  with features or smaller crates. This needs deliberate API boundaries and
  validation of every package; do not strip runtime functionality for build speed.
- Compare opt-level 2/3, codegen-unit counts, and thin LTO against frame timings
  before changing the fast profile further. Keep full LTO opt-in.
- sccache is not installed. Its Rust cache does not support incremental builds;
  do not replace this working incremental loop blindly. It is a candidate for
  nonincremental cold/CI builds, not a proven improvement here.

Sources: [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html),
[Cargo timings](https://doc.rust-lang.org/cargo/reference/timings.html),
[sccache Rust support](https://github.com/mozilla/sccache/blob/main/docs/Rust.md).

## Desktop rendering and memory, Midget (2026-10-08)

History, shell 0.4.12 (current shell memory and CPU figures are in `budgets.toml`, for example shell PSS under 75 MiB at 0.5.14).
Shell 0.4.12 shares immutable window pixels between the desktop cache and Wayland
submission instead of cloning them. Quiet external windows reuse their last image;
Wayland slots accumulate damage independently until released. Files 1.0.3 paints
selection bands and skips unchanged draws. Resize keeps mapped buffer dimensions
until replacement buffers arrive. Focused regression tests cover these paths.

Matched four-window snapshots (Store, Settings, Remote, Files): shell PSS
249 MiB before sharing, 228–234 MiB afterward; compositor about 130 MiB. These are
snapshots, not a peak-memory benchmark.

Earlier rendering comparison: median shell draw 34.6 → 11.95 ms; p95
49.2 → 37.6 ms; idle shell CPU 5.59 → 3.74% of one core. Files reported average
draw median 3.6 → 1.1 ms. Input cadence differed because of SSH/host load, so
these results do not establish input latency or throughput. Raw results are in
`out/render-performance/{before,after}.{json,log}`. Reproduce with
`tools/midget-render-measure.py LABEL 'External(N)'` after enabling trace and
opening the same windows. Hardware GLES compositing is active on Intel HD 530;
shell/SDK rasterization is still CPU work. No claim of 100% GPU rasterization.

### Demand-allocated window SHM (0.4.13)

Four windows on Midget: Wayland SHM allocation 59.85 → 47.77 MiB, including
unchanged desktop buffers. Buffer count 10 → 7. Each quiet window avoids a
second image until a busy buffer blocks a changed frame. Shell PSS increased
217 → 236 MiB across the restart (245 MiB after resize), so this measurement supports lower SHM
allocation, not lower total shell RAM. Native browser/session state also changed.
Equal-area Files reshape 1200x825 → 1100x900 was visually checked after update.

## Frame-time regression tests (host)

Six desktop scenes are timed on the host at 1920x1080 (Files at 1200x1920, the window body), each as a forced repaint
of the whole shell or of the Files page. Every scene prints `BENCH frame_<scene>_median` and `BENCH frame_<scene>_p95`
in ms, and its test fails when the 95th-percentile frame is over **16.7 ms** (one 60 Hz frame):

| Scene | Test | What is timed |
| --- | --- | --- |
| Desktop idle | `bench_frame_desktop_idle` | desktop, no windows |
| Settings open | `bench_frame_settings_open` | Settings window open |
| Window menu open | `bench_frame_menu_open` | Window menu popup open over the desktop |
| External app pixels | `bench_frame_ext_pixels` | The frame an app's new pixels ask for: the windows only, no bar, Dock or wallpaper |
| Window drag | `bench_frame_window_drag` | Settings dragged by its title bar; each frame is one pointer move plus the repaint |
| Files, 5,000 items | `bench_frame_files_5k` | Files folder page with 5,000 rows (`springboard/src/files/bench.rs`) |

Scenes live in `springboard/src/shell/frame_budget.rs`; the budget is `FRAME_BUDGET_MS` there.

Run them (from the repo root):

```sh
python3 tools/bench.py --frames --no-record   # only the frame scenes; exit 1 names the scene over budget
python3 tools/bench.py --frames               # same, and appends the keys to docs/perf-history.jsonl
python3 tools/bench.py --no-record            # every host bench, including these
```

Or directly, in `springboard/`:
`cargo test --profile fast --features desktop-wayland,testkit --lib bench_frame_ -- --ignored --nocapture`.
The window-menu and window-drag scenes need `desktop-wayland`; without it they are skipped.

Other benches: `bench_ui_*` (kit widgets, `bifrost-ui`), `bench_files_*` (`bifrost-files`) and the render crate's `glass_cost`;
`tools/bench.py --midget` measures the device against `docs/budgets.toml` (idle CPU and wakeups, PSS, threads, sizes, RAM),
`--midget --graphics` adds the window-server suite (`tools/graphics_bench.py`), `--audio` the audio latency bench
(`bifrost pc audio-bench`, [audio-latency.md](audio-latency.md)), and `--history` prints `docs/perf-history.jsonl`.

### Idle redraw on the device (Midget, `top` and the frame trace)

An idle desktop must cost almost nothing, with an app in front too. Measure it with the app doing what it does when it
does nothing (Music on its Radio page, a station selected, playback failing is the worst case seen):

1. Turn the shell's frame trace on (`bifrost pc midget trace on`; off again with `trace off`) and read the shell log for
   10 s with no input: `springboard: frame draw N present N damage N% gap N why <cause>`.
2. Count the `springboard: frame` lines of those 10 s in the shell log (`bifrost pc log`, filtered to the window of time).
   An idle desktop draws a frame once a second at most, for a changed CPU or RAM reading
   (`why ...:bar-reading`), and the clock once a minute. More than that names its cause in `why`: `ext-frame` is the app in
   front sending pixels, `dirty` is a full desktop frame asked for by something else, `bar-reading` and `perf-hud` are the
   partial frames.
3. Read `top -b -n 2 -d 5` for the second sample: the shell (`springboard`), the app and `bifrost-wm`. Idle, the shell
   is under 5% and the app under 5%; `BIFROST_FRAME_STATS=1` in the app's environment adds a line a second with the app's
   frames, the repaints it did not send because they equalled the screen, and (Music) which source asked for the repaint.
4. `slow frame ... damage=0%` lines mean a whole desktop frame came out unchanged: the work that was wasted is the
   `draw=` figure.

Host check of the same paths: `bench_frame_ext_pixels` (above) and `cargo test --lib --features desktop-wayland
a_frame_for_an_apps_pixels_alone_draws_no_shell_surface identical_state a_bar_only_frame` in `springboard/`.

These run the same drawing code as the shell, but on the host CPU, not on the lab PC's Intel GPU. A pass here does not
prove the Midget frame time: check the device numbers with `tools/bench.py --midget --graphics`.
The budget is a ceiling, not a tuned target. Lower `FRAME_BUDGET_MS` only after a measured run shows headroom; never
raise it to make a regression pass.
