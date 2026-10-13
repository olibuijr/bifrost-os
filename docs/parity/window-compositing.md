# Window compositing (slice: window-compositing)

Round 75. Goal: the desktop stays smooth on the lab PC (Intel GPU). Changes remove redundant GPU work and
backdrop rebuilds. No Apple matrix rows are closed by this slice (the matrix rows are not edited here).

## What changed

| Change | Code | Test |
|---|---|---|
| A window whose drawable lies inside the opaque inset of a still window above it is not drawn or uploaded by the GPU presenter. Its pending damage is kept and drawn when it shows again. | `springboard/src/gpu/occlusion.rs` (pure geometry), `render_windows` in `springboard/src/bare/wayland.rs` | `gpu::occlusion::tests` |
| `WindowFrame.opaque`: shell-drawn windows are opaque inside their corner radius (no vibrancy); client windows are not declared opaque and never cover others. | `springboard/src/bare/wayland.rs` (struct), `springboard/src/shell/desk.rs` (frame sites) | build-checked by the lead |
| Glass (material) windows are always drawn; glass between a hidden window and its cover keeps the hidden one drawn, because the glass blurs what lies below it. Animating windows (fx not identity) are neither hidden nor covers. | `gpu::occlusion::hidden` | `gpu::occlusion::tests` |
| A window's cover only counts once it has shown content (`frame.pixels`). | `render_windows` | by reading |
| A stack change rebuilds only the glass backdrops above the first changed position, not all of them. | `Presenter::set_window_stack`, `stack_change_from` in `springboard/src/gpu/wl.rs` | `gpu::wl::stack_tests::a_stack_change_only_dirties_the_windows_above_it` |

## Already in place (checked, not changed)

- Unchanged windows are not re-rendered: `draw_desk` reuses `window_cache` for unchanged, quiet windows, and
  `Presenter::draw` redraws only the owed areas of each drawable.
- Moving a window damages no content: the presenter only changes the stack and the live backdrop.
- The GPU window shadow is a shader pass drawn only when the drawable owes a redraw (`set_shadow` owes the whole
  drawable on change), so it is already cached per drawable.

## Not done (and why)

- Shadow cache per window size in the CPU path (`draw_window`, `shadow = true`): this path runs only without the
  Wayland presenter. A cached shadow image needs an alpha-aware blit whose semantics were not checked here, and the
  round rules forbid building to verify it. Left for a round that can compile.
- Skipping shell-side rendering of hidden windows (`draw_window` in `draw_desk`): the shell still renders them; only
  the GPU upload and draw are skipped. A desk-level skip would need the same occlusion data before the frame is built.

## What still needs checking on Midget

- Drag a window across a maximized window and back: no flicker, and the hidden window appears complete when uncovered.
- Glass windows (Settings sidebar, Dock vibrancy) above and below a moving window: the blur still follows the window.
- Minimize, restore and close while another window covers the target: the effect still plays.

## Evidence

No reference files were read for this slice; the work is measured against the existing presenter and shell code
(`springboard/src/gpu/wl.rs`, `springboard/src/bare/wayland.rs`, `springboard/src/shell/desk.rs`).
