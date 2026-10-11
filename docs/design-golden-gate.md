# Design decisions from macOS Golden Gate (2026-10-08)

Source material for the BifrOSt PC desktop. Golden Gate is macOS 27 (developer betas from June 2026,
release fall 2026). Use these notes to decide what the desktop does, and state the evidence each time.

## Sources

| Source | What it gives us | Limits |
|---|---|---|
| Macworld, "macOS Golden Gate in pictures: 5 design upgrades" (2026-06-12) | Sidebar shading, Liquid Glass transparency setting, fewer menu icons, icon contrast | Beta 1 only |
| 512 Pixels, "Golden Gate Beta 6 Turns Stoplights into Liquid Glass" (2026-08-17) | Traffic-light glass, background windows drop highlights, active colour too pale | Commentary, not Apple docs |
| 512 Pixels screenshot library (2026-09-14) | Full screenshot set, Tahoe comparisons | Images are not fetched into this repo |
| Apple WWDC25 "Build an AppKit app with the new design" and "Build a SwiftUI app with the new design" | Liquid Glass rules for toolbars, sidebars, controls | Videos, read as summaries |
| Apple HIG (developer.apple.com/design/human-interface-guidelines) | Layout, sidebars, toolbars, menus, materials | JavaScript-rendered: could not be read here; decisions rely on the search snippets and the WWDC material |

## Decisions

1. **Traffic lights.** Focused lights keep the system colours (red, yellow, green) and carry a thin
   glass rim. Background lights are a neutral grey with no rim. Beta 6 was criticised for highlights on
   background windows, so the rim is only on the focused window. Tokens: `TRAFFIC_RIM`, `TRAFFIC_INACTIVE`
   in `render/src/ui.rs`. Status: shipped in 0.4.50; visually checked on Midget 0.4.51 (focused Settings
   window). Background-window state not yet checked on the device.
2. **Window and menu corners.** One radius per surface family, named: `WINDOW_RADIUS_PT` (10),
   `MENU_RADIUS_PT` (8). Golden Gate made window corners consistent across the OS, so new surfaces take a
   named token, not a literal. Status: shipped in 0.4.50.
3. **Window colours.** `WINDOW_BAR_FOCUSED` and `WINDOW_BAR_BLURRED` replace literals. Status: shipped.
4. **Menus.** Golden Gate removes icons from items that do not need them. BifrOSt menus have no icons,
   so nothing to change. Keep it that way unless an item needs a glyph to be found quickly. Since 0.4.68 menus show key equivalents in a trailing column (⌘W, ⇧⌘Z), as macOS does, and size to their content.
5. **Sidebars (done in the kit, 0.4.84).** Golden Gate shades the whole sidebar column and runs it to the top of the
   window under the traffic lights. The SDK `source_list` draws it (accent glyphs, neutral bold selection) and an app
   asks for the full-height window with `window = "full-size"`. Files 1.13 is the first app; the rest follow the
   batches in `docs/golden-gate-sdk-adoption.md`.
6. **Liquid Glass transparency (not done).** Golden Gate adds Settings > Appearance > Liquid Glass to set
   how clear the glass is. BifrOSt already has Reduce Transparency (opaque fallback in
   `render/src/gfx.rs`). A clear/tinted slider is a later step; it needs the GPU blur path first.
7. **Icons (not done).** Apple added outlines and more contrast to system icons. Check the BifrOSt
   dock icons against that before the next icon pass.

## Shipped after 0.4.51

- 0.4.52/0.4.53: hover-revealed traffic-light glyphs, title-bar double-click zoom, shaded sidebar column (`SIDEBAR_FILL`), no status values in sidebar rows. Glyphs and zoom checked only by unit test and code; the sidebar shade is subtle on Midget.
- 0.4.54: `blur_material` (reduced-size blur) for glass and menus, Cmd+Q / Cmd+H. Host benchmark: Spotlight glass 12.7 to 4.7 ms.

- 0.4.56: GPU-first desktop (see `docs/gpu-first.md`): wallpaper and glass on the GPU, dock/bar/menus/Spotlight glass from one GPU blur.

## Golden Gate release notes that drive the next rounds (9to5Mac 2026-09-14, Six Colors review)

- Liquid Glass slider (clear to tinted) in Appearance: `bifrost_render::glass::set_clarity` exists, Settings UI pending.
- Traffic lights in Liquid Glass with a bounce on press: needs per-layer transforms (GPU layers).
- One corner radius across windows (done: `WINDOW_RADIUS_PT`).
- Fewer menu icons (BifrOSt menus have none).
- Edge-to-edge sidebars and more distinct active windows: sidebar column tint done; inactive windows should recede more (title, shadow).
- Blue sidebar glyphs in Finder: Files sidebar icons use `LINK` tint.

## Shipped in 0.4.59

- Inactive windows recede: `BACKGROUND_DIM` (14 %) darkens the body of every window that is not in front, on top of the dimmer title and traffic lights. Checked on a Midget screenshot with Settings behind Files.
- Soft shadows under the dock and menus (`soft_shadow` in `shell/desk.rs`), cached per size; `bench_chrome_effects` records `dock_shadow_1080p`, `menu_shadow_1080p`, `window_dim_800x600`.
- SDK `Material` (sidebar, titlebar, popover, HUD) for apps: see `docs/bifrost-ui.md`.

## Added in iteration 5 (reviews read 2026-10-09: Six Colors, MacRumors "50 new features", MacStories)

- Toolbars are discrete bars again, not floating glass buttons: better shading, more defined buttons, a unified bar with
  headings and controls (Six Colors, comparing Photos in Tahoe and Golden Gate). SDK `widgets::toolbar` draws it.
- Sidebars: attached to the left edge, darker fill, coloured icons (done: `SIDEBAR_FILL`, SDK `widgets::source_list`).
- Show Borders (Accessibility > Display) outlines the boundaries of controls and panels where translucent surfaces
  blend into the window. Implemented as the existing Button Shapes flag, renamed, and now sent to every app.
- The Dock indicator turns light grey for an app that quit but still runs a background process (not built).
- Mission Control and Spaces animations are smoother; windows are restored to their places when a display returns
  (not built; relevant to "scale to any screen").
- The comparison screenshot (Six Colors, `two-windows-gg-tahoe-6c.jpg`) shows capsule groups in the toolbar (back and
  forward, share and more, search) and coloured folder icons in the sidebar.

## Open questions

- Hover-revealed traffic-light glyphs (close, minimise, zoom on hover): not built; needs pointer hover
  on the desktop.
- Whether the rim reads as glass on a real panel at 165 Hz. Check on Midget with a background window.

## Performance benchmarks and history

`tools/bench.py` runs the `#[ignore]`d benchmarks (`glass_cost_*` in render, `bench_*` in the shell) with the
`fast` profile, prints the change against the previous run (flags >15 % as REGRESSION) and appends to
`docs/perf-history.jsonl`. `tools/bench.py --history` prints each series. Baseline at 0.4.55 (host): desktop frame
at 1080p with two windows 14.6 ms dirty and 14.0 ms idle (an idle frame costs as much as a dirty one, so
damage tracking is the next big win), glass 5.0 ms, material blur 3.0 ms against 8.0 ms exact.
Next profiling step: `perf record` on the `bench_desktop_frame` test binary (run it in the background, it hung a foreground call).

## Next iteration queue

- Idle desktop frames: skip the redraw when nothing is dirty (profile first).
- Cache rendered text strings (window titles, dock labels).
- Sidebar tint is subtle on Midget; compare with a Golden Gate screenshot.
- Check the traffic-light glyphs on Midget (0.4.56 makes them bolder; 0.4.55 drew them too faint to see).

## Third-party window chrome (labwc)

Apps that are not ours (the ArtCraft apps) get server-side title bars from labwc. `packaging/arch/wayland/rc.xml`
puts the buttons on the left (close, minimise, zoom), centres the title (13 pt bold) and uses the 17 pt window radius.
The `BifrOSt` labwc theme ships in `packaging/arch/wayland/themes/{light,dark}/labwc/` (themerc plus 12 px
traffic-light SVGs, grey when inactive); Light is installed at `/usr/share/themes/BifrOSt/labwc/`. On every
Light/Dark change `shell/labwc_theme.rs` copies the matching variant into the session user's
`~/.local/share/themes/BifrOSt/labwc/` and sends labwc SIGHUP. Our own apps (`is.olibuijr.bifrost.*`) keep drawing their own chrome.
