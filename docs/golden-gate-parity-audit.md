# Golden Gate parity audit (2026-10-10)

> Status 2026-10-13: Round 1 is in the code (`CAPSULE_PT` 36, `TOOLBAR_GLYPH_PT` 20, `SIDEBAR_GLYPH_PT` 20 in
> `sdk/src/widgets.rs`; `LIGHT_GAP_PT` 9 in `src/shell/desk.rs`; Files `SIDEBAR_W` 160; SDK `with_dot`, `folder_icon`,
> `search_button`). The labwc title bar in Round 2 is gone: `bifrost-wm` draws third-party title bars. In the "Open after
> round 2" list, the frozen-snapshot Control Center is fixed (the desktop column now clears only its own rect and floats
> over the live desktop, `draw_desk_center` in `src/shell/notify/center.rs`) and window vibrancy is the window
> server's job (`docs/window-server.md`; the shell-side live backdrop was deleted). The Spotlight light-mode text and
> the Software Update rollback were not re-checked. The measurements below are history.

Measured against `goldengate_refimages/` (2x captures: 1 pt = 2 px) and a Midget screenshot of 0.4.91
(shell and apps draw at `ui.s` = 1.25 px/pt on 2560x1440). Values are points. Ruler check: the reference traffic
light is 27 px = 13.5 pt; ours is 12 pt.

## Round 1 targets

| # | Surface | Reference | Ours (0.4.91) | Change |
|---|---|---|---|---|
| 1 | Traffic lights | 14 pt, 23 pt pitch | 12 pt, 20 pt pitch | `LIGHT_PT` 14, `LIGHT_GAP_PT` 9 |
| 2 | Toolbar capsule (SDK `toolbar`) | 36 pt high | 32 pt | `CAPSULE_PT` 36 |
| 3 | Toolbar glyph | ~18 pt visible ink | ~14 pt | `TOOLBAR_GLYPH_PT` 20 |
| 4 | Sidebar glyph (SDK `source_list`) | ~19 pt ink, accent | ~13 pt | `SIDEBAR_GLYPH_PT` 20 |
| 5 | Sidebar width (Finder) | 158 pt | 208 pt | Files `SIDEBAR_W` 160 |
| 6 | Sidebar sections | Recents (no header); Favorites: Applications, Desktop, Documents, Downloads; Locations: home, disk, Network, Trash; Tags with colour dots | Favorites (Home, …, Recently Deleted); Locations (Recents, Tags, Computer) | regroup; SDK row gains a colour-dot leading mark (`with_dot`) |
| 7 | Icon view | folder art: two-tone body, lighter back tab, embossed glyph for special folders; 12 pt label; 64 pt icon default | flat one-colour folder, 10 pt label | SDK `folder_icon` (drawn, any size) + label `MAC_CALLOUT_PT` |
| 8 | Status bar | hidden by default (View > Show Status Bar) | always on, two rows | hidden by default, one row when shown |
| 9 | Sidebar toggle by the lights | absent | circular button | remove (View > Hide Sidebar, ⌃⌘S) |
| 10 | Search | round magnifier button that expands to a field | always a field | SDK `search_button` → field on click |
| 11 | Menu bar | transparent over the wallpaper, text with a soft legibility shadow; open title on a light glass capsule | opaque bar; open title on a solid blue rect | transparent bar, glass capsule highlight |
| 12 | Menus | 6 pt gap under the bar; highlight is an accent capsule inset 5 pt, radius 8; hairline separators inset 12 pt; optional leading SF symbol per item; shortcuts in secondary label colour, glyphs spaced | flush to the bar; heavy separators; no item symbols | match; `Row` gains an optional symbol |
| 13 | Dock | Finder first, divider, Downloads stack and Trash at the end; lighter glass with a bright 1 px rim; ~24 pt radius | no Trash or divider; grey tint | add divider + Trash; glass rim; radius |

## Later rounds

Window shadow depth and radius, Control Center, Notification Center, Spotlight blur, Launchpad, list and column
views, Get Info, Quick Look. Each needs its own measured pass against the matching reference image.

## Round 2 (found browsing Midget, 0.4.92)

| Surface | Problem | Change |
|---|---|---|
| Notes, Calendar, Reminders | iPad ports: always dark (ignore Light), legacy title row, iOS navigation (back chevrons, large titles, bottom toolbar, Reminders tile grid) | Golden Gate kit: `window = "full-size"`, `source_list` sidebar, kit `toolbar`, `Pair` colours |
| Native windows (Browser) | labwc title bar (since replaced by `bifrost-wm`), always-dark egui chrome, white bands, square corners | Browser on the SDK with a WKWebView-parity `WebView`; native apps follow appearance through `bifrost-portal` |
| Stacking | reopening a running SDK app did not raise it over a native window | fixed in 64ff3540 (foreign-toplevel activate) |
| Status menu extras | CPU/RAM text readouts | Wi-Fi, battery, Control Center, clock only (readouts move to Activity Monitor) |
| Apple menu | system items live in the app menu | a leading system menu (About This Computer, System Settings…, Sleep, Restart, Shut Down, Lock Screen); app menu: About <App>, Settings…, Hide, Quit |

## Open after round 2 (lead review)

- Control Center / Notification Center draw over a frozen screen snapshot (`notify/center.rs` `draw_desk_center` copies `frost.sharp`): windows behind stop updating and bar-edge artefacts (thin arcs under the menu extras) show. Draw both as live overlays from `mod.rs`.
- Window vibrancy (history, now `bifrost-wm`): `bare/wayland.rs` rendered windows with no backdrop or glass shapes, so sidebars/toolbars cannot blur the wallpaper; GPU shadow radius is `margin*0.36` instead of `m::WINDOW_RADIUS`; shadow reach follows the title height.
- Spotlight field text (`textfield.rs`) stays white in Light.
- Software Update through Settings rolled back 0.4.92 (ssh install works); the guard output is now logged (51b98355) to find the cause on the next run.
