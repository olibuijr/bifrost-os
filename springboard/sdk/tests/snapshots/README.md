# Kit control snapshots

Golden images for the SDK kit controls (`../snapshots.rs`): button, checkbox, switch, slider, stepper, popup,
segmented, text field, progress, section card, each in Light and Dark at scale 1.0 and 1.5625.
Files are `<control>-<light|dark>-<scale>x.png` (uncompressed RGB PNG, openable anywhere).

- Compare: `cargo test -p bifrost-app --test snapshots`. A pixel differs when any channel is off by more than 2;
  the test fails when more than 0.1% of pixels differ. A missing golden is written and passes (prints `blessed`).
- On failure `<name>.actual.png` and `<name>.diff.png` (red = changed) are written next to the golden (git-ignored).
- Bless after an intended visual change: `BIFROST_BLESS=1 cargo test -p bifrost-app --test snapshots`, review the
  changed PNGs, commit them.
- Appearance is forced with `appearance::jump_to` (no Auto, no cross-fade); controls are drawn on a fresh window
  with no ticks, so output is deterministic. All cases run in one test because the appearance is process-global.
