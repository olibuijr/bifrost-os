# UI overhaul todo (Settings, light/dark, kit)

Branch `feat/settings-golden-gate`. **No build, check, test or publish until the owner says OK.** Everything below the
"Done in code (uncompiled)" heading has been edited but never compiled.

## New: dock apps are System Apps, installed at first setup

Owner request. Every app pinned in the default dock must ship with the system, be categorized **System App**, and be
installed during initial setup (not fetched later from the App Store):

- Files, Clock, Contacts, Photos, BifrOSt Browser (plus the rest of the default pinned set).
- Category: today manifests use `category = "System"` with `kind = "system"` (see `store/install.rs` tests); decide
  whether the user-facing label is "System App" and apply it in the App Store listing and in Settings > Storage.
- Installer: `springboard/src/installer/plan.rs` and `packaging/arch/wayland/apps` list what is installed; add these
  apps there so a fresh install has them. A System App cannot be uninstalled from the App Store (Remove is hidden).
- Default dock: `shell/desktop_dock.rs` pinned list must only name apps that exist on a fresh install.
- Apps to audit: `is.olibuijr.files.desktop`, `is.olibuijr.browser.desktop`, Clock, Contacts, Photos (the last three
  may still be built-in shell apps; decide built-in vs packaged).
- Verify on a clean Midget install: dock icons present, category shows System App, no App Store download needed.

## Done in code (uncompiled)

- `springboard/ui/src/prefs_layout.rs`: measured Settings geometry as a pure model + tests (this one was compiled and
  its 6 tests passed before the no-build instruction; constants were changed afterwards and are untested).
- `render/src/appearance.rs`: light/dark scheme engine (eased 280 ms fade, `Pair`, `Mode`, Auto by hour, publish_mode).
  Tests existed and passed (4) before later additions (`fading`, `publish_mode`, `mode_label`) which are uncompiled.
- `render/src/gfx.rs`: `Surface::blit_colored` (white image tinted by a colour, for text that fades with the scheme).
- `render/src/ui.rs`: `Pair` colour tokens (SIDEBAR_BG, DETAIL_BG, CARD_BG, SEPARATOR_C, LABEL, LABEL_2, SELECTION,
  ROW_PRESSED, FIELD_BG, WINDOW_FRAME_*).
- `shell/desk.rs`: Settings is a full-size-content window (no title bar, traffic lights inside the sidebar, 17 pt
  corners blended on all four corners, 740x690 pt default), `Metrics.pt`, `title_of`, scheme-following frame.
- `shell/list.rs`: 232 pt sidebar with 32 pt rows / 30 pt selection plates / 20 pt icons, account block without a
  plate, 13 pt text, 49 pt card rows, 13 pt card radius, scheme colours, `Pane.status_pt`, sidebar search frame.
- `shell/icons.rs`: `tile_sized`. `shell/settings_search.rs`: pill field, 13 pt, scheme colour.
- `shell/settings.rs`: fixed sidebar, detail margin 20, scheme fills, sidebar cache keyed on scheme, live sidebar
  search filter, `APPEARANCE_PREF`, `Pick::Appearance` handler.
- `shell/pages.rs`: Displays > Appearance page (Light / Dark / Auto). `shell/mod.rs`: `appearance_tick`, frame
  scheduling while fading, startup scheme without a fade.

## Known issues to fix before the first build

1. Existing tests that pin old values will fail: `list.rs` pressed row equals `HIGHLIGHT`; row height / radius
   asserts; `settings.rs` sidebar width tests; desk tests that use `m.title` for Settings; any test constructing
   `Pane { .. }` or `Metrics { .. }` without the new fields (`status_pt`, `pt`).
2. `Pane` struct literals elsewhere (outside `settings.rs`) need `status_pt`.
3. `list.rs` still imports `CELL`, `SEPARATOR`, `HIGHLIGHT`-based items that may now be unused (warnings must stay at zero).
4. `SIDEBAR_FILL`, `SIDEBAR_SELECTED`, `WINDOW_BAR_*` are now partly unused in shell code; remove or keep for apps.
5. DONE (follows `row_rect`): Sidebar touch/hover hit areas used the 44 pt row geometry in `settings.rs` `sidebar_touch`; check they use the
   32 pt pitch. `row_centre`/`sidebar_centre` helpers (used by tests and Midget tap-window) follow `row_rect`.
6. DONE in code: Search: the sidebar field's touch routing (`settings.rs` near the `search.touch` call) still uses the list's own
   frame in the detail pane; it must use the sidebar's frame and the sidebar's local x. Physical keyboard input must
   reach the field (the docked on-screen keyboard is a tablet leftover).
7. DONE in code (unbuilt): Detail pane: the back/forward pill (forward stays dimmed, no forward history yet), the header
   card (icon, bold title, description, first-level pages only) and General-page row glyphs (`shell/row_glyphs.rs`). Other
   detail pages still have no row glyphs; the old note was: The nav bar still draws "< Previous" and an inline title. Spec in `docs/settings-layout-spec.md`.
8. Light/dark coverage: Settings, window frame, shared list rows, the menu bar and menus (`BAR_TINT`, `MENU_TINT`,
   `MENU_SOLID`) and the dock tint (`DOCK_TINT`) now follow the scheme (unbuilt). Still hard-coded dark: dock labels and
   badges, Spotlight, control centre, lock screen, notifications, every
   built-in app (Terminal, Photos, Files, Clock, Calendar, Notes...), the `controls.rs` chevron/check images, `NavBar`
   title text, slider and switch tracks, `icons` tile backgrounds that assume a dark list.
9. Apps in separate processes do not receive the scheme (`Ctx::appearance()` reads process statics). Needs a
   `ToShell`/`FromShell` message and an SDK field, then docs in `docs/bifrost-ui.md`.
10. Text baked with `WHITE` is tinted at blit; text baked with `SECONDARY`/`BLUE`/`RED` is not and keeps one colour in
    both schemes. Check contrast of `SECONDARY` and `BLUE` on the light fills.
11. Corner blend in `copy_rect_rounded` reads the pixels under the corners; with the GPU presenter / wayland frame
    path the window may be composited differently (`WindowFrame`, `clip_corners`). Verify on Midget.
12. DONE (text only): Radius rule: `ui.rs` said "every radius is at most 6, nothing is a pill". Settings now uses 13/17 pt radii and a
    pill field. Update the rule text, the `CARD_RADIUS_PT`/`CELL_RADIUS_PT` tokens and AGENTS.md "Design" section.
13. `bifrost-hig` review (LAY, CHR, TYP, COL, CTL) has not been run on the new screens. Fix list: TYP-01 no literal
    sizes (several `Weight`/size literals remain), COL-01 named constants only, COL-05 contrast in both schemes.
14. Dark palette: Midget is dark; the dark `Pair` values reuse the old tokens. Re-measure the dark set against a
    Golden Gate dark reference when one is available (`goldengate_refimages/` has the light General page only).
15. `goldengate_refimages/` (406 MB) is untracked; decide LFS or external storage before committing.
16. Settings "real options": audit every row in `pages.rs` as setting-backed, read-only or dead; remove or wire dead
    rows; test each page.
17. Release: bump `springboard/Cargo.toml` + `CHANGELOG.md`, run the full test suite once, `bifrost pc publish`, then
    the owner runs the OTA from Software Update. Do not publish before the owner's OK.
18. Naming: rows and labels that use Apple product names (AirDrop, Siri-style names) should get our own names; the
    owner asked for the UI without the Apple branding.
19. Row glyphs exist only on the General page; add them to Displays, Sounds, Network, Keyboard, Privacy and the rest.
20. `Glyph` drawings (`row_glyphs.rs`) are simple first drafts; check each at 23 pt against the reference on Midget.
21. `cargo check --all-targets` passed once (exit 0, 3 warnings, now removed) before the no-build instruction; every edit
    after that is unbuilt.
