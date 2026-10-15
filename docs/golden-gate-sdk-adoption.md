# Golden Gate (macOS 27) UI SDK adoption

> Status 2026-10-14: Preview now uses the shared Golden Gate toolbar and publishes its visible buttons as controls;
> its page-thumbnail gallery remains custom because rows have document-specific heights. The blank-launch compliance
> fixture does not require Preview's document-contextual sidebar. Batches 1 and the rounds after it moved apps onto the kit. `bifrost pc adoption` now reports
> 28 of 44 apps with no fixed dark colours, and 41 of 44 use `window = "full-size"` (not `keyboard`, `maps-desktop`,
> `photobooth-desktop`). The tables below are the 2026-10-10 history; the live count comes from the command.
> The reference captures (`goldengate_refimages/`) are not in the repository.

Goal: apps get the Golden Gate look from the SDK (`bifrost_app::widgets`, `bifrost-ui`) instead of drawing their own
chrome. Work goes in batches of 10 issues. A batch is done when every issue meets its acceptance, the batch is released
(AGENTS.md "Release every iteration") and checked on Midget in Light and Dark.

Tools: `bifrost pc adoption [--json]` (the per-app baseline below), `bifrost pc gallery [DIR]` (every kit widget in
both schemes), `bifrost pc smoke APP [--light]` (the real app binary on the host), and on Midget
`bifrost pc midget tap-window "External(N)" X Y`. Skills: `bifrost-pc` (workflow), `bifrost-hig` (rules GG-01 to GG-05).

References: `goldengate_refimages/27-Golden-Gate-Finder-Home.png`, `-Finder-Search.png`, `-Notes.png`,
`-Reminders.png`. Rules: `docs/design-golden-gate.md`, skill `bifrost-hig`.

## Starting point (2026-10-10, system 0.4.83, Files 1.12.1)

- SDK widgets: `material`, `large_title`, `section_label`, `toolbar`, `source_list`, `text_field`, `popup_button`,
  `menu`. Only `text_field` follows Light and Dark; the rest use the fixed dark tokens (`WHITE`, `FILL`,
  `SIDEBAR_FILL`, `SIDEBAR_SELECTED`, `WINDOW_BAR_FOCUSED`, `SEPARATOR`) and `Material` tints are dark only.
- Adoption: only Files calls the SDK (`text_field`, 3 call sites). The other 19 apps under `springboard/apps` draw
  their own sidebars, toolbars and fields with fixed colours (counted by grep: 11 to 69 uses each).
- Patterns in every reference: full-height sidebar with the traffic lights inside it and no separate title row;
  toolbar buttons as glyphs in light glass capsules; a search capsule with a magnifier; sidebar rows with a tinted
  glyph, trailing count, and a grey selection plate with a bold label; small grey section headers.

## Batch 1: the kit, then Files as the pilot

| # | Issue | Acceptance |
|---|---|---|
| 1 | **Widget gallery preview.** One ignored test renders every SDK widget in Light and Dark to PNG (`PREVIEW_DIR`). It is the check for each later issue. | `preview_sdk_gallery` writes `gallery-light.png` and `gallery-dark.png`; compared against the references |
| 2 | **Every SDK widget follows the scheme.** `toolbar`, `source_list`, `popup_button`, `menu`, `large_title`, `section_label` and `Material` tint/solid use `Pair` tokens (`.now()`). Add the missing named `Pair`s in `render/src/ui.rs`. | No fixed dark token left in `sdk/src/widgets.rs` except for text on accent fills; gallery correct in both schemes |
| 3 | **`search_field` (NSSearchField).** A capsule with a leading magnifier, placeholder, a clear button when not empty, and a focus ring. Hit-testing for the clear button is in `bifrost-ui`. | Matches `Finder-Search.png`; unit tests for clear hit-testing; `docs/bifrost-ui.md` entry |
| 4 | **Toolbar item groups (NSToolbarItemGroup).** Glyph buttons (symbol name, not text) grouped into light glass capsules with hairline dividers; a selected state for segmented groups (view mode). | Matches the Finder toolbar (back/forward pair, view group); overflow `»` still works; tests on the layout model |
| 5 | **Source list, Golden Gate style.** Rows take an optional symbol glyph tinted with the accent colour; the selection is a grey plate with a bold label (9 pt radius, `prefs_layout` `m::*`); headers are subhead semibold grey; counts trailing. | Matches the `Finder-Home.png` and `Notes.png` sidebars in both schemes |
| 6 | **Full-size content windows for SDK apps.** An app can ask for a unified window (no title row; the traffic lights sit in the sidebar column; the toolbar shares the top row). Needs a manifest or protocol flag and shell support (Settings already does this in `shell/desk.rs`). | Files opens without the separate "Files" title row; window drag works from the toolbar's empty area; older apps unchanged |
| 7 | **Sidebar toggle and sidebar header buttons.** A toolbar button beside the traffic lights collapses or shows the sidebar (`NSSplitViewController.toggleSidebar`), with View > Show/Hide Sidebar and ⌃⌘S. | Collapse and restore work in Files; menu item and shortcut wired; split model test |
| 8 | **Path bar (Finder path bar).** A footer widget with folder glyphs and chevrons, clickable segments, truncation from the middle. | Matches the footer of `Finder-Search.png`; segment hit-testing tested |
| 9 | **Files adopts issues 2 to 8.** Sidebar glyphs per location (Recents clock, Desktop, Documents, Downloads, Home house, Computer drive, Trash), search capsule, grouped toolbar, unified window, path bar; drop the "Folder" caption under grid items. | Midget screenshot in Light and Dark side by side with `Finder-Home.png`; `bifrost-hig` review recorded |
| 10 | **Adoption guide and checker.** A section in `docs/bifrost-ui.md` on how an app moves to the kit, plus a `tools/` script that counts fixed dark tokens and SDK calls per app, to track the remaining apps. | Script output recorded here as the batch 2 baseline |

## Batch 2 candidates (choose 10 when batch 1 ships)

Notes (three columns: sidebar, list, detail), Reminders (smart-list tiles), Music and Podcasts adoption; inspector
popover with grouped rows and toggles (`Reminders.png` right panel); `switch` and `segmented_control` widgets;
grouped form rows (`NSForm`-style, shared with Settings); context menus from the SDK `menu`; sheets and alerts;
tag chips (`Notes.png`, `Reminders.png` "All Tags"); Liquid Glass clarity setting reaching app materials.

## Progress

### Batch 1: done (2026-10-10, system 0.4.84, Files 1.13.1)

| # | Status | Evidence |
|---|---|---|
| 1 | Done | `preview_sdk_gallery` (`sdk/src/widgets.rs`); `~/Pictures/bifrost/gallery-light.png`, `gallery-dark.png` |
| 2 | Done | No fixed dark token left in `sdk/src/widgets.rs` except `WHITE` on accent fills; materials use `MATERIAL_*` pairs; widget tests compare against `.now()` |
| 3 | Done | `widgets::search_field`, `bifrost_ui::search` (2 tests) |
| 4 | Done | `ToolbarItem::icon/in_group/selected/custom`, `Toolbar::layout_with` (2 model tests, 2 widget tests) |
| 5 | Done | `SidebarItem::with_symbol`, neutral bold selection (widget test) |
| 6 | Done | Manifest `window = "full-size"`; `desk.rs` `full_size`, `drag_zone` (test); the Wayland window frame's drag height and no-drag zones (now handled by `bifrost-wm`, `docs/window-server.md`); checked on Midget (no title row, lights in the sidebar, toolbar buttons in the band clickable). Dragging by the band on the compositor path was **not** exercised: the control socket's taps go through the shell, not the compositor. |
| 7 | Done, with a gap | Sidebar button and ⌃⌘S (`SplitView::toggle` exists; Files keeps its own flag). No View menu item: apps cannot add menu items yet (`docs/bifrost-ui.md`, parity gaps). |
| 8 | Done | `widgets::path_bar`, `bifrost_ui::pathbar` (3 tests); Files footer segments navigate (test) |
| 9 | Done | Files 1.13.x: Midget screenshots `~/Pictures/bifrost/files-1.13-light.png`, `-dark.png`, `-nosidebar.png` |
| 10 | Done | `docs/bifrost-ui.md` "Golden Gate kit" (API, steps, gaps); `tools/sdk-adoption.py` |

`bifrost-hig` notes on Files 1.13 against `27-Golden-Gate-Finder-Home.png`: LAY full-height sidebar and unified bar
match; CHR traffic lights inside the sidebar match; CTL toolbar capsules, segmented view group and search capsule
match; COL sidebar glyphs in the accent, neutral selection; TYP title 15 pt bold, rows 13 pt. Differences left: grid
folder art is the flat folder, not Finder's glyph folders; the sort control is a glyph button, not a pull-down.

### Batch 2 baseline (`bifrost pc adoption`, 2026-10-10; 20 apps then, 44 on 2026-10-13)

```
app                    fixed pairs  full-size  widgets
books-desktop             13     0  no         -
browser                    0     0  no         -
calculator-desktop        12     0  no         -
calendar-desktop          39     0  no         -
clock-desktop             29     0  no         -
contacts-desktop          29     0  no         -
daw                       27     0  no         -
files                      0    48  yes        path_bar, search_field, source_list, text_field, toolbar, toolbar_band
freeform-desktop           6     0  no         -
games-desktop             15     0  no         -
keyboard                   2     0  no         -
music-desktop             51     0  no         -
news-desktop              17     0  no         -
notes-desktop             39     0  no         -
podcasts-desktop          18     0  no         -
reminders-desktop         31     0  no         -
remote                     7     0  no         -
shortcuts-desktop         13     0  no         -
voicememos-desktop        25     0  no         -
weather-desktop           15     0  no         -

1 of 20 apps use the kit with no fixed dark colours
```

## Keeping adopted apps adopted (app compliance gate)

Adoption is enforced, not just tracked. `tools/check-app-compliance.py` (acceptance step "app compliance") fails when an
app regresses against `docs/app-compliance-baseline.json`: hand-rolled sidebar, toolbar, search or path bar (GG-02 to
GG-05), a window that is not `full-size` (GG-01), fixed dark tokens or hard-coded white/black fills (COL-02, COL-03),
literal colours, sizes, radii and resolutions. Apps with a `tests/compliance.rs` (generated by `tools/gen-app-compliance-tests.py`) run the real binary in Light and Dark at
two scales and compares a golden, so a kit change that moves a widget shows up as a snapshot diff in every app that uses
it. When you change a kit widget or SDK API, run `python3 tools/check-app-compliance.py --changed-sdk` and re-run the
listed apps' compliance tests (bless intended changes with `BIFROST_BLESS=1`). Full description: `docs/app-compliance.md`.
