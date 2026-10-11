# BifrOSt build and iteration performance

Measured on Titan, 2026-10-07. First desktop update: 0.3.18.

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

- `bifrost build --host` uses optimized `fast`, not full LTO. Desktop publishing
  already defaults to this profile; retain that default. It uses opt-level 3,
  32 codegen units, incremental compilation and the existing lld linker.
- Package only the shell (`--bin shell`), rather than all binaries or apps.
- Strip the downloadable copy, retaining symbols in the local binary. The first
  update is 4,640,999 bytes (approximately 4.4 MiB); assets are already installed.
- Publish a validated artifact with `bifrost pc publish --skip-build` to avoid even
  another Cargo no-op. Normal publishing still builds before packaging.
- Build/publish prints separate phase times, and builds generate Cargo HTML
  timing reports under `springboard/target/cargo-timings/`.
- Test desktop code with relevant filters and its desktop preview.

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
