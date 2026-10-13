# Boot task 02: `bootweave-frame`, one boot frame for every stage

> Status 2026-10-13: the progress bar described here was dropped on 2026-10-12; the frame is the mark alone and render_bar is a no-op (docs/boot-architecture.md, One frame). The rest stands.

Contract: `docs/boot-architecture.md`, "One frame". Task: `tasks/boot/02-bootframe.md`. Branch `boot/02-bootframe`.
Not built or tested by the agent (owner rule: the lead builds once after every wave is merged). Everything below is
checked by reading the code against the APIs it calls.

## What is implemented

`springboard/bootweave/frame` (crate `bootweave-frame`, `no_std` plus `alloc`, no dependencies):

- **Frame description.** `Frame { width, height, appearance, progress, screen }`, `Appearance` (Dark default, Light),
  `Screen` (`Boot`, `Unlock { name, dots, failed }`, `Options { entries, selected }`, `Rollback`, `StartFailed { selected }`,
  `RestartedAfterProblem`, `ShuttingDown`). `Frame::boot(w, h)` is the loader's frame: Dark, bar at 0.
- **Renderer.** `render(&Frame, out, stride, PixelOrder)` draws the background, the BifrOSt mark, the progress bar and the
  screen's shapes. `PixelOrder::Bgrx` (`0xffRRGGBB`, GOP BlueGreenRed, DRM XRGB) and `PixelOrder::Rgbx` (`0xffBBGGRR`).
  `render_bar` redraws only the bar. `palette`, `bar_rect`, `mark_box` are the geometry and colour tokens.
- **Mark.** Vector, from `packaging/brand/bifrost-mark.svg` as data: stroked polylines with butt ends and round joins,
  even-odd filled polygons, 2x2 supersampling. Scaled from the panel's short side (`160 * h / 1440`, at least 16 px),
  centred, 110 px above the middle at 1440 lines. The same numbers the old loader and `bootsplash::paint` used.
- **Progress bar geometry.** `240 * h / 1440` wide, `max(4 * h / 1440, 2)` high, 110 px below the middle. Track and fill
  colours per appearance (Dark track 58,58,60; fill 229,229,234).
- **Hand-off.** `Handoff::new(w, h, appearance, order)` renders the frame once. `compose(ui, out, now, boot_ms)` fills the
  bar over `FILL_MS` (300 ms, ease-out) from `first_fill(boot_ms)`, then cross-fades into the screen over `FADE_MS`
  (450 ms, smoothstep). `fade`, `fade_weight`, `first_fill` are pure and exported.
- **Full-bar frame precomputed** in `Handoff::new` (task review fix): the fade no longer clones the framebuffer each tick.

Callers moved onto `bootweave-frame` (the old splash paint, fade and Handoff are deleted):

| File | Change |
|---|---|
| `springboard/bootweave/efi/src/main.rs` | the loader paints `Frame::boot` through `render` and one full-screen GOP `BufferToVideo`; the old logo blit and the bar `VideoFill` are gone |
| `springboard/bootweave/efi/Cargo.toml`, `Cargo.lock` | depends on `bootweave-frame` by path (lock entry added by hand: `bootweave-frame` in `bootweave-efi`'s dependencies) |
| `springboard/src/bare/kms.rs` | `boot_fade` holds `bootweave_frame::Handoff`; `Handoff::new(w, h, Dark, Bgrx)`; `compose` takes `f32` times |
| `springboard/src/bin/installer.rs` | first KMS frame is `render(Frame::boot)`; the fade uses `bootweave_frame::fade` and `FADE_MS` |
| `springboard/wm/src/drm.rs` | the first scanout's boot buffer is `render(Frame::boot)` |
| `springboard/wm/src/boot.rs` | `FADE_MS` and `splash_alpha` come from the frame crate (`fade_weight`); the file still includes `src/bootsplash.rs` for `mark` only |
| `springboard/src/bootsplash.rs` | now only the boot milestone log (`MARKS`, `boottime_ms`, `mark`) |
| `springboard/bootweave/efi/logo.bgrx` | deleted (`git rm`) |
| `tools/bifrost_brand.py` | no longer writes `logo.bgrx` (otherwise the deleted file comes back on the next run). Outside task 02's files; kept because it is the direct consequence of the deletion. Revert it if the lead disagrees |
| `springboard/Cargo.toml`, `springboard/Cargo.lock`, `springboard/wm/Cargo.toml` | path dependency on `bootweave-frame`; lock entries added by hand |

## How it maps to the contract

| Contract item | State |
|---|---|
| One description and renderer, `no_std` plus `alloc`, no dependencies | done |
| Background, mark scaled from the short side, bar geometry from the panel size | done |
| Screen variants `Boot`, `Unlock`, `Options`, `Rollback`, `StartFailed`, `RestartedAfterProblem`, `ShuttingDown` | the variants exist; the shapes for `Unlock` (field, dots, red ring), `Options` (capsules), `StartFailed` (two buttons) are drawn; `Rollback`, `RestartedAfterProblem`, `ShuttingDown` draw the mark and bar only (text, see todo) |
| BGRX and XRGB writers, no floating point beyond `f32`, no allocation in `render` | done (`Rgbx` is the XRGB8888 order; `render` and `render_bar` do not allocate) |
| Used by loader, Bootweave, installer and window server | loader, installer, `bifrost-wm` first scanout and KMS hand-off done. Bootweave (task 06) and the window server's reveal (task 09) call the same API |
| Delete the old splash paint, fade and Handoff, and `logo.bgrx`; keep `bootsplash::mark` | done |
| Light and Dark follow the user's appearance | `Appearance` is passed through; the value comes from the timing record (task 03) |
| `unsafe` only in existing UEFI and KMS calls, each with a SAFETY comment | no new `unsafe` in this task |

## Tests (worked out by hand, not run)

- `lib.rs`: `first_fill` (0, 1700/2300, clamp 0.95), `fade_weight` (0.5 symmetric, 0.25 gives 0.84375), `fade` at k 0 and 1,
  `Frame::boot` defaults, the hand-off timeline at 640x360 (bar x 290, 60 px wide, rows 207 and 208), and the hand-off
  frame at zero fill equal to `render`.
- `render.rs`: bar and mark geometry at 1920x1080, 2560x1440 and 640x360; pixel packing per order; the progress fill
  (0.5 fills 90 of 180 px); stroke hit (butt ends, round joins); even-odd fill (rune holes, star stem); mark layers and
  the arch's hollow; every screen deterministic in both appearances at both sizes; the unlock field's dots and red ring;
  the start-failed accent on the selected button.
- `wm/src/boot.rs` (existing): `splash_alpha` now delegates to `fade_weight`.

## Left, and what the lead must verify

Todo left (not in this task's scope or not verifiable without a build):

1. **Golden snapshots.** The task asked for golden images per screen at 1920x1080 and 2560x1440, Light and Dark. No image
   files are committed: the golden bytes need a build to produce. The current tests check determinism and the corner
   colour only. Add `sdk/tests/snapshots`-style goldens after the first build.
2. **Glyphs.** Labels for the unlock name, option entries, button labels and the failure texts are not drawn. They need
   the boot glyph set (a fixed bitmap font in `bootweave-frame`, no dependency). Until then `Rollback`,
   `RestartedAfterProblem` and `ShuttingDown` show only the mark and bar.
3. **Unlock and options input.** Only the shapes exist; the input and the passphrase flow are task 10.
4. **Contract text.** `docs/boot-architecture.md` (Where BifrOSt is) and `docs/window-server.md`, `docs/bifrost-boot-design.md`
   still name `bootsplash::paint`. Those lines should point at `bootweave-frame::render`. Not edited here (the contract is the lead's).
5. **`progress` and `timing`** are the skeleton's modules, owned by tasks 03 and 06. Their bodies may still be `todo!()`; the
   frame path (`render`, `Handoff`) does not call them, and `lib.rs` re-exports their types unchanged.

Verification after the lead builds (host, then Midget):

1. Host: `cd springboard && cargo test --offline -p bootweave-frame` (all `frame` tests), `cargo test --offline -p bifrost-wm boot`
   (the `splash_alpha` tests), `cargo build --offline --profile fast --bin installer --features desktop-wayland` (installer
   and KMS callers). The loader: `tools/build-efi-boot.sh` (builds `bootweave.efi` for `x86_64-unknown-uefi` with the new lock).
2. `python3 tools/check-names.py` (passes in this branch).
3. Midget: publish, update, then `python3 tools/midget.py recovery` must say `phase=accepted`. Then:
   - `bifrost pc midget screen /tmp/boot-frame0.png` during the loader (or the first KMS frame) and compare the first frame
     with `bootweave_frame::render(Frame::boot(w, h))` from a host test that writes the same buffer.
   - `tools/boot-bench.py installed --runs 5 --record`: check `desktop-first-ui` and `desktop-faded` marks still appear
     in `/run/bifrost/boot-marks` with the 450 ms fade, and that no text reaches tty1 (journal only).
   - Confirm the loader's frame (`bootweave.efi` on the ESP) is the new mark: no blank or old logo at power-on.
