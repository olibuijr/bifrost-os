# Tab bar controller and bar appearance

Branch `parity/team-p-tab-bar`, built on the `arm` branch. Not yet verified on the tablet.

Covers two rows of the tablet platform parity data (`docs/parity-matrix/ipados/data/`):

- `uikit-view-controllers.json`, "Tab bar controller" (`NativeTabBarController`, `NativeTab`, `NativeTabGroup`,
  `NativeSearchTab`): partial.
- `uikit-appearance-animation-layout.json`, "Bar and appearance proxies" (`NativeNavigationBarAppearance`,
  `NativeTabBarAppearance`, `NativeAppearance`): partial.

Both are models. The renderer that owns a window draws them; the shell and the SDK do not draw these bars yet.

## What was built

### Tab bar controller: `springboard/ui/src/tab_controller.rs`

- `Tab` is a screen (`TabKind::Screen`), a group of screens (`TabKind::Group`), or the search tab (`TabKind::Search`).
  Groups do not nest. A tab can be disabled; a disabled screen is drawn but cannot be selected.
- `TabBarController` holds the tabs and one selected screen. The first selectable screen is selected at creation.
  Group ids are not selectable; the screens inside a group are.
- Width decides the chrome (`Presentation::for_width`). From `SIDEBAR_MIN_WIDTH_PT` (768 points) on, the sidebar is
  shown. Below it the tab bar is shown along the bottom, `TAB_BAR_PT` (49 points) high.
  - Compact width: screens and groups share the bar evenly. The search tab is a square at the trailing end. A group
    bar item opens its screens as a list (`open_group`, `group_screens`, `close_group`). Choosing a screen in the list
    selects it and closes the list.
  - Wide width: the sidebar is a `SourceList` (`springboard/ui/src/sidebar.rs`) that is `SIDEBAR_WIDTH_PT` (280
    points) wide. Each group is a header above its screens. Rows are `SIDEBAR_ROW_PT` (36 points) and headers
    `SIDEBAR_HEADER_PT` (28 points). The sidebar reuses the source list, so its drawing and hit testing are shared.
- `layout(bounds)` returns the content rectangle and the bar or sidebar rectangle.
- Keyboard and pointer both go through the same selection:
  - `step(backwards)` moves to the next or previous selectable screen and wraps around.
  - `select_index(n)` selects the nth selectable screen (0-based), for Command-1 to Command-9.
  - `tap(bounds, x, y)` returns `Tap::Selected`, `Tap::OpenedGroup` or `Tap::Missed`.
- `is_search_selected()` tells the app to show its search field.
- `set_tabs` replaces the tabs and keeps the selection when it is still valid. Otherwise the first screen is selected.

### Bar appearance and the appearance proxy: `springboard/ui/src/bar_appearance.rs`

- `NativeBarAppearance` holds the background (`Default`, `Opaque` with a colour, or `Transparent`), the separator
  colour, and the tint. Each field is optional.
- `NativeNavigationBarAppearance` adds the title and large-title text attributes. `NativeTabBarAppearance` adds the
  normal and selected item states, each with an icon colour and title attributes.
- `NativeAppearance` is the standard appearance. `update_standard` and `read_standard` read and write the
  process-wide value. A bar with its own appearance draws with it over the standard one, field by field
  (`resolve_navigation`, `resolve_tab`). A bar without one draws with the standard one as it is.
- Colours are packed RGBA, as the renderer packs them.

### SDK surface: `springboard/sdk/src/tabs.rs`

Re-exports the types above under the names an app uses. `springboard/sdk/src/lib.rs` declares the module.
`springboard/ui/src/lib.rs` declares the two new model modules.

## How an app uses it

```rust
use bifrost_app::tabs::{Tab, TabBarController, Tap};

let mut tabs = TabBarController::new(vec![
    Tab::screen(1, "Library").with_symbol("books.vertical"),
    Tab::group(2, "Playlists", vec![Tab::screen(20, "Mix"), Tab::screen(21, "Old")]),
    Tab::screen(3, "Settings"),
    Tab::search(4, "Search"),
]);

// Window geometry: the content area and the bar or sidebar.
let layout = tabs.layout(window_bounds);
// Draw the selected screen in layout.content.

// Pointer.
match tabs.tap(window_bounds, x, y) {
    Tap::Selected { .. } | Tap::OpenedGroup(_) | Tap::Missed => {}
}

// Keyboard.
tabs.step(false);       // Tab
tabs.select_index(0);   // Command-1
```

Appearance is set once for the whole app, and bars pick it up:

```rust
use bifrost_app::tabs::{read_standard, update_standard};

update_standard(|s| {
    s.navigation.bar.configure_with_opaque_background(0xFF_F2_F2_F7);
    s.navigation.title.size = Some(17.0);
    s.tab.selected.icon_color = Some(0xFF_00_7A_FF);
});
let standard = read_standard();
```

## Tests

Unit tests are in `#[cfg(test)]` modules of both model files. They cover:

- selection rules: groups and disabled tabs are refused, `step` wraps, and `select_index` follows presentation order;
- the width switch between bar and sidebar, and the layout rectangles of each;
- bar item rectangles, the square search tab, and taps on bar items, group bar items, sidebar rows and headers;
- `set_tabs` keeping or moving the selection;
- appearance merging, the configure helpers, and the process-wide standard (one test restores the default).

These tests have not been run. The lead builds and runs them.

## Desktop behaviour

The model takes only window bounds and pointer or keyboard calls. A desktop window that is wide gets the sidebar, and
a narrow one gets the bar. Command-number and arrow keys work the same way on both. Nothing here depends on touch.

## Not yet done

- The shell window and the SDK do not draw the bar or the sidebar. Until they do, apps cannot see these controllers.
  The drawing belongs to the renderer that owns the window (`springboard/src/shell`, with the SDK kit).
- The bar's group list is a model state (`open_group`). Presenting it as a popover or sheet is left to the renderer.
- The search tab only reports that it is selected. The search field and its results are not connected.
- Shell bars (navigation, tab) do not read the standard appearance when they draw. They still use the shared kit
  styles.
- Verification on the tablet and on the desktop is still to do.
