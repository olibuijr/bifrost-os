# Notifications and menu bar extras: smoothness (slice `notification-perf`, round 75)

Scope: keep the desktop smooth on the lab PC (Intel GPU). Banners, stacks, menu bar extras and Control Centre
should do no repeated work when nothing changed. No reference SDK rows are closed by this slice; the matrix
rows are untouched. No macOS 27 reference files were read.

## Done in code (host-testable, not yet on Midget)

| Area | BifrOSt path | State |
|---|---|---|
| Menu bar label widths and fitted names | `springboard/render/src/cache.rs` (`TextCache::width`, `TextCache::fit`), used by `springboard/src/shell/desktop_bar.rs` (`measure_chrome`, `fit_chrome`) | Done. The bar used to shape about a dozen labels with `Ui::measure`/`Ui::fit` on every draw and on every hit-test and popup-rect call. These are now memoised in the chrome text cache, keyed by size and text-setting epoch, and bounded at 2048 entries. |
| Desktop Control Centre tile labels | `springboard/src/shell/control_center.rs` (`draw_desk_cc`, `draw_media_tile`) | Done. Titles and subtitles of the Wi-Fi, Bluetooth, Focus, Display and Sound tiles and the Now Playing title and artist were rasterised every frame, including all of the opening animation. They now come from the shared chrome text cache. |
| Tests | `render/src/cache.rs` (`widths_and_fits_match_the_ui_and_are_remembered`, `widths_follow_dynamic_type_and_the_metric_cache_is_bounded`), `src/shell/desktop_bar.rs` (`chrome_cache_tests`) | Added. Not run: the lead builds. |

## Measured by reading, left as they are

- Menu bar clock and battery: `Shell::set_info` returns early when the `Info` is unchanged, and re-renders the clock
  and percentage only when the minute or the level changes. The bar clock needs no wake of its own.
- Notification banner: the card is rendered once in `start_banner`, and `draw_banner` only blits it. The frame
  still repaints everything while the banner slides (`notify_damage_all`). A partial repaint of the banner rectangle
  is the next step, but it touches the clipped home path and needs Midget checking first.
- Lock-screen list: rebuilt only when the store generation, the minute or the battery key changes
  (`notify_prepare`). Per-card text in `ListView::new` is still uncached (it has no access to the shell's cache).
- Control Centre on the desktop opens without a snapshot (`open_desk_control_center`). The phone-style sheet still
  blurs a snapshot of the screen on open (`snapshot_screen`); that path is not the lab PC's.

## What still needs Midget checking

- Banner slide smoothness on the Intel GPU with the desktop (frame times during the 280 ms slide).
- Menu bar redraw cost with the CPU and RAM readouts on (the labels change every sample, so each new string adds one
  cache entry until the cache starts over).
- Control Centre opening: the first open should draw the same text as before (the cache is cleared on a Dynamic Type,
  bold or contrast change, and on an emoji font reload).
