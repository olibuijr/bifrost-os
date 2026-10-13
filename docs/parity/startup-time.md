# Desktop startup time (slice startup-time)

Goal: fewer blocking steps on the UI thread between process start and the first frame, with the same
behaviour once the desktop is up. No parity matrix rows are involved; this is a performance slice.

## Done

- Installed-app icons (PNG) decode in parallel, one thread per app, before the home tiles are built.
  `springboard/src/shell/apps.rs` `decode_icons`, used by `Shell::new` in `springboard/src/shell/mod.rs`.
  Before, `apphost::icon_image` decoded each installed app's icon in sequence on the UI thread.
- The wallpaper catalogue (`wallpapers()` in `springboard/src/bin/shell.rs`) builds on a worker thread
  instead of before the first frame. It reads the full `wallpaper.raw` on every start and crops up to six
  Pictures stills on first use. The loop hands the list to the shell on its next turn (`try_recv` at the
  top of the loop). Only Settings > Wallpaper reads it.

## Measured

Not measured: the slice ran without a build (the lead builds and profiles). The expected win is the sum of
the installed-icon PNG decodes and the wallpaper catalogue read, both removed from the path to the first frame.

## Still on the UI thread before the first frame (candidates)

- `fontfallback::install_from_dir` scans the font library and parses every face (`render/src/fontobj.rs`
  `scan_dir`, `render/src/fontfallback.rs`). It cannot move to a background thread without a redraw trigger,
  because text drawn before it finishes would show `.notdef` for characters only the fallback faces cover.
- `Shell::set_data_dir` (quick search, notifications, today, screentime, prefs, home layout) runs before the
  first frame.
- `dock_background` blurs the dock's wallpaper crop, and `fit_wallpaper` covers a still on a cache miss.
