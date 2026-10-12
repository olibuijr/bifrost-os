# Wire 4: content-area fill in Dark for Settings and the App Store

## What is wired

Settings' detail pane and the App Store main area filled with `DETAIL_BG`, which is pure black in Dark. They now fill
with `CONTENT_BG` (`springboard/render/src/ui.rs`: dark content grey `pack(30, 30, 30)` in Dark, white in Light), as
native content views do. Only main content fills changed. Sidebars, grouped cards (`CARD_BG`) and cells are unchanged.

| File | Change |
|---|---|
| `springboard/src/shell/settings.rs` | import `CONTENT_BG` instead of `DETAIL_BG`; `PageView::draw` and `draw_entry` fill the pane with `CONTENT_BG.now()` |
| `springboard/src/shell/store/mod.rs` | import `CONTENT_BG` instead of `DETAIL_BG`; `bg()` returns `CONTENT_BG.now()` (used by the store's top bar and pane fill) |

No test or snapshot names `DETAIL_BG` in these files, so no expectation changed. The appearance guard in `ui.rs`
checks only for `BLACK` and `CELL` fills, so it is unaffected.

## How a user reaches it

Switch the PC to Dark (or Auto, which is Dark after 19:00), then open Settings and any page with a detail pane, or
open the App Store. The main area is dark grey, not black.

## How to verify on Midget

1. `bifrost pc midget appearance dark`
2. `bifrost pc midget open-app Settings`, then open a detail page (for example Display).
3. `bifrost pc midget screen <scratch>/settings-dark.png`: the detail area is about RGB 30,30,30 and the sidebar keeps
   its own tone.
4. `bifrost pc midget open-app AppStore`, then `bifrost pc midget screen <scratch>/store-dark.png`: the main area is
   dark grey.
5. `bifrost pc midget appearance light`: both areas must be white.

Host preview: `bifrost pc smoke Settings` and `bifrost pc smoke AppStore`, each with and without `--light`.

## REA and official evidence

- `python3 -I tools/parity/docc.py appkit/nscolor/controlbackgroundcolor` returned the official description: "The color
  to use for the background of large controls, such as scroll views or table views." This is the basis for
  `CONTENT_BG` (reused from wire 3, evidence `ev_f82ebd8719732e253617f0fa4a235954eb9b61fb56c2c20aeaeb04b009140b94`).
- No new REA page capture was made in this round; the change is a token swap, not a new reference behaviour.

## Remaining gaps

- Photos (`springboard/src/shell/photos/mod.rs`) fills its grid, nav bar and viewer with literal `BLACK`, not
  `DETAIL_BG`. The viewer uses black by design (the guard allowlists `shell/photos/`). Changing the grid background
  would also change the viewer behind the photo, so it is left for a separate change that splits the two.
- Other shell app modules that fill with `DETAIL_BG` (`preview_panel.rs`, `docpanel.rs`, `list.rs`) were not
  changed; they are not in this round's scope.
- No Midget screenshot was taken; the lead verifies on Midget after the build.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path |
|---|---|---|
| appkit-user-interface | Appearance Customization | springboard/render/src/ui.rs (`CONTENT_BG`), applied in springboard/src/shell/settings.rs and springboard/src/shell/store/mod.rs |
