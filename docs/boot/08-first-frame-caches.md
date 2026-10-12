# Boot 08: caches for the first desktop frame

Task `tasks/boot/08-first-frame-caches.md` (phase 3, depends on 01). Contract: `docs/boot-architecture.md`, "Prepared,
not probed". Status: code written and read-reviewed; not built, not run (the lead builds once after the waves merge).

## What is implemented

One cache file, `/var/cache/bifrost/first-frame/first-frame.ffc`, in `springboard/src/first_frame_cache.rs`. It holds:

| Part | Built from | Used by |
|---|---|---|
| Wallpaper, cover-fitted to the panel | the wallpaper still (`wallpaper.raw` or `wallpapers/<id>.raw`) | `shell.rs` takes it as the shell's wallpaper instead of cover-scaling the still at start |
| Icons of `assets/icons`, at the Dock's icon size | `assets/icons/*.raw`, resized to `layout.icon` for the panel and scale | `shell.rs` icon loader: `cached_icon(name, size)`, falling back to the decode |
| Menu bar glyph atlas | `SFUI.ttf`, printable ASCII at 13 pt times the scale | built and stored; **no consumer yet** (see "Left") |

File format: magic `BFFC`, version 1, key (panel w, h, scale in thousandths, FNV-1a hash of the wallpaper file), counts,
then the blocks, little-endian `u32` pixels as `Image` uses. The decoder is bounds-checked: a wrong magic or version,
a truncated block, a dimension or count over the limit, a bad name, or trailing bytes is `None`, and the shell decodes
as before.

### Key and validity

The key is the panel size, the layout scale and the wallpaper's content hash. A cache is used only when all three match.
The scale is an input, never read from global state inside the cache module:

- The shell calls `activate` after `layout::set_adaptive` and `set_user_scale` (`bin/shell.rs`), so `activate` reads
  `layout::base_scale`, the same scale the shell's `Layout::new` uses for its Dock.
- The installer and the updater (`springboard --first-frame-cache`) have no adaptive state, so they use
  `installed_panel()`: the first connected DRM connector's mode and EDID size (bytes 21 and 22, in centimetres) and
  `layout::auto_scale` of those. A scale chosen in Settings is not known there. Then the shell misses on its first start,
  rebuilds in the background, and every later start hits. The cache heals itself.

Writes are atomic (`first-frame.ffc.tmp`, then rename). A miss in the shell starts a background build (`spawn_build_here`),
and so does a wallpaper change in Settings.

### Where the hooks are

| Hook | File | What it does |
|---|---|---|
| Installer | `springboard/src/installer/plan.rs`, `install`, after `mkinitcpio` | `installed_panel()` plus `build_into` into `/mnt/var/cache/bifrost/first-frame`; a failure is logged and skipped |
| Updater | `packaging/arch/recovery/guard.rs`, `install`, after the `pacman -Su` of the shell | runs `springboard --first-frame-cache` (best effort), which calls `rebuild_default` (invalidate, then build) |
| Shell start | `springboard/src/bin/shell.rs`, before `Shell::new` | `activate` loads the cache into a `OnceLock`; the wallpaper and the icon closure read from it |
| Wallpaper change | `springboard/src/bin/shell.rs`, `Effect::Wallpaper` | `spawn_build_here` for the new still |
| `--first-frame-cache` | `springboard/src/bin/shell.rs`, first branch of `run` | the updater's entry point; returns before any display setup |

No `unsafe` was added. The file I/O is std only; `bootweave-frame` is not touched and is not a dependency of this module.

## How it maps to the contract

- "At install, update, wallpaper change and resolution change": install (`plan.rs`), update (`guard.rs`), wallpaper change
  (`shell.rs`). **Resolution change is not hooked** (see "Left").
- "The wallpaper decoded at the panel size, the Dock and menu bar icons rasterised at the panel scale, and the menu bar
  font's glyph atlas, in `/var/cache/bifrost/first-frame/`": the wallpaper and the Dock icons are done; the menu bar atlas
  is built but not consumed.
- "The shell and the chrome clients load them for their first frame and fall back to decoding": the shell does (wallpaper
  and Dock icons, through `cached_icon` and `wallpaper()`). The chrome clients do not read the cache yet.

## Tests (host, written by hand; not run)

`first_frame_cache.rs` `mod tests`: `fnv1a` against the published FNV-1a vectors (empty and `"a"`); `parse_drm_mode`
(first line, whitespace, zero, missing `x`, oversized); `edid_size_mm` (bytes 21 and 22 times 10; short or zero block);
`wallpaper_path` (default, named, `..` and `/` refused); `key_for` scale in thousandths (1.5625 gives 1563) and wallpaper
hash; `icon_px` (64 at 1.0, 100 at 1.5625, from `Layout::build`'s `zpx(ICON_PT)`); encode and decode round trip; decode
rejects wrong magic, version, truncation and trailing bytes; decode rejects an oversized icon count (offset 28) and
an oversized width (offset 36) without allocating; key mismatch (panel, scale, wallpaper) is a miss; icons found by name
and size; a cache file loads only for its own panel, scale and wallpaper, and `invalidate` removes it. That is 12 tests
for the pure logic and the file round trip.

## Left

- **Menu bar glyph atlas has no consumer.** `desktop_bar.rs` should draw its labels from `first_frame_cache`'s glyph
  list when present. Not done: that file is outside this task's ownership, and the task's hook is minimal.
- **Chrome clients** (the Dock and menu bar processes, if separate) do not read the cache; the shell does.
- **Resolution change** is not hooked. The shell rebuilds only on a wallpaper change; a mode change at runtime has no
  rebuild, so the first start after it misses and rebuilds. Hooking the display-change event belongs with the window
  server (task 09).
- **Settings scale** is not in the installer or updater's key. The shell's own rebuild fixes it on its first start.
- **Wallpaper hash cost.** Every shell start reads the whole wallpaper file and hashes it to check the key. This is cheap
  next to the cover scaling, but it is a full read; a stored mtime and size check would avoid it. Not measured.
- **The shell's cache decode is per pixel** (`from_le_bytes`), not a bulk copy. Measure it on Midget before tuning.

## Midget verification (for the lead)

Preconditions: the release with this change is published and updated (`bifrost pc midget update --restart`), and
`python3 tools/midget.py recovery` says `phase=accepted`. Then:

1. Confirm the installer or updater wrote the cache: `ssh midget 'ls -l /var/cache/bifrost/first-frame/'`. Expect
   `first-frame.ffc`, no `.tmp` file. After the first shell start the file should match the shell's key.
2. Confirm the key matches the shell's scale: restart the shell and check the journal for `first-frame cache:` errors
   (`ssh midget 'journalctl -b -u springboard'`). No error line means the build ran; a second start must not rewrite the
   file (its timestamp stays the same).
3. Cold boot timing, the budget: `python3 tools/boot-bench.py installed --runs 5 --record` (task 01's bench). Compare the
   `desktop-first-frame` mark against the baseline from task 01. Expect it to move earlier, not to be identical; record
   both numbers in `docs/perf-history.jsonl`.
4. Miss path: delete the cache (`ssh midget 'sudo rm -f /var/cache/bifrost/first-frame/first-frame.ffc'`), restart the
   shell (`ssh midget 'systemctl --user restart springboard'` or reboot), and confirm the file reappears after the
   first start. The first frame must still come up (fallback works).
5. Wallpaper change: switch the wallpaper in Settings and check that the file is rewritten (`ls -l` timestamp).
6. Screenshot: `bifrost pc midget screen first-frame.png` after the desktop is up, to confirm the wallpaper and
   Dock icons draw correctly from the cache (no fuzzy or misplaced icons, the wallpaper fills the panel).

Verification this round was by reading only. Nothing was compiled, run or booted.
