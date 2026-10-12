# Wire 4: desktop menu close repaint, and menu bar tooltips

Branch `wire4/menu-close`, from main 0.5.40. Not built or run here (no builds in this round; the lead builds once).

## What is now fixed

**Bug: a thin line of a closed menu stayed under the menu bar.** After a menu bar menu closed (Escape, or a click
elsewhere), a strip of its top shadow stayed drawn in the 6 pt gap between the bar and the popup until something
repainted there.

Cause, from `springboard/src/shell/desktop_bar.rs`: the GPU desktop frame clears and redraws only the rectangles in
`desk_regions_now()`, and the frame after a close clears the rectangles the previous frame recorded (`desk_clear`,
`shell/mod.rs`). The open popup's rectangle started at the popup's top edge `y`. The shadow of the popup reaches
`SHADOW_REACH_PT` (10 pt) above its top edge (`desk::soft_shadow`), so its upper part, in the gap from the bar's bottom
edge down to `y`, lay outside every rectangle. Nothing cleared it on close.

Fix: the rectangle is now computed by a pure function, `popup_damage_rect` (desktop_bar.rs). It extends the popup by
the shadow reach on the left, right and bottom, and by the same reach upward, clamped to the bottom of the menu bar
(the bar already repaints its own strip). The rectangle is stored in `desk_regions` while the menu is open, so it is
the rectangle cleared on the close frame. No change to `bare/wayland.rs`, the proto crate or the menu chrome id.

## How to verify on the test PC (Midget)

1. Open any app-less desktop state (Home), so the desktop bar and wallpaper are visible.
2. Click the App menu (or any bar menu, for example Window) so its popup opens.
3. Press Escape. Then take `bifrost pc midget screen /tmp/menu-closed.png`.
4. Expected: nothing remains of the popup's shadow between the bar and the wallpaper, including the strip directly under
   the bar. Repeat with a click on the wallpaper to close instead of Escape.
5. Before the fix, the strip stays visible until the wallpaper or the bar repaints (for example by moving the pointer
   over the dock).

Host check: `cargo test --offline -p springboard menu_damage` (the `menu_damage_tests` module; not run in this round).

## Host tests

`springboard/src/shell/desktop_bar.rs`, module `menu_damage_tests`, with a 28 px bar and a 12 px reach:

| Case | Input (popup x, y, w, h) | Expected rectangle (x, y, w, h) |
|---|---|---|
| Menu below the bar | (100, 34, 200, 150) | (88, 28, 224, 170): top 22 clamped to 28, right 312, bottom 198 |
| Lower popup, top shadow in the gap | (10, 70, 100, 80) | top 58 (the shadow row 58 is damaged) |
| Left edge | (5, 34, 100, 40) | (0, 28, 117, 60): left clamped to 0, right 117 |
| Containment | (300, 34, 180, 220) | the rectangle contains the popup on every side |

## Tooltips for menu bar status items: not wired (no model)

The TipKit "Tooltips" row (`docs/parity-matrix/data/tipkit.json`) is Partial. Its Apple reference is `NSView.toolTip`,
"The text for the view's tooltip." (docc: `appkit/nsview/tooltip`). No tooltip model exists in `springboard/ui/src`,
`core`, `render` or `sdk`, apart from the `ToolTip` material (`sdk/src/widgets.rs`), and the menu bar status items
carry only a label and an action (`Action::StatusItem`). With no model to wire, no tooltip UI was added. The row stays
Partial.

Gap left: a hover label for bar items and bar status items needs a tooltip model first (text, hover delay, placement
under the bar, dismissal on click), then a shell hover state. Not started here.

## REA evidence

- `ev_99564747f9b2afc1eb1eb66eb02d0f32e15176cf1ad67033394c766266265f86` (rendered page, `appkit/nsview/tooltip`, first capture)
- `ev_70562afddc2f543700b785c55465109998a1ebf9c2186eaf32d52e33fac802d9` (rendered page, `appkit/nsview/tooltip`, second capture)

## Remaining gaps

- Tooltips: no model for menu bar items or any control (see above). The matrix row `tipkit` / `Tooltips` stays Partial.
- The shadow reach is a fixed 10 pt plus 2; a scale change is covered because the values pass through `ui.pt`.

## Matrix rows

No matrix row changes status in this round. The row this area touches:

| slug | feature (as in `docs/parity-matrix/data/tipkit.json`) | BifrOSt path | Status after this round |
|---|---|---|---|
| tipkit | Tooltips | springboard/sdk/src/widgets.rs:47-48 (Material::ToolTip) | partial (unchanged; no tooltip model) |
