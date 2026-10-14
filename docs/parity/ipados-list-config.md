# Tablet list configurations and reserved regions

Status: built on parity/team-p-list-config; not yet verified on the tablet. The matrix rows are
"Configurations" (list content and background configuration) and "Reserved regions" in
`docs/parity-matrix/ipados/data/uikit-appearance-animation-layout.json`. Both are `partial`.

## What was built

All three modules are pure models with unit tests. They have no dependency on the renderer, the shell or the window
manager, so the same code serves the tablet and the desktop. The SDK re-exports the UI crate as
`bifrost_app::controls`, so an app reaches them as `controls::list_content`, `controls::background_configuration` and
`controls::reserved_region`.

| Module | Contents |
|---|---|
| `springboard/ui/src/background_configuration.rs` | `ConfigurationState` (highlighted, selected, disabled), `StateValue<T>` (one value per state with fallback to `normal`), `BackgroundConfiguration` (fill, stroke, stroke width, corner radius, edge insets) and `resolve(frame, state)` returning the painted frame and values |
| `springboard/ui/src/list_content.rs` | `TextStyle` (headline, body, subhead, caption, with point sizes at a text scale), `ListContentConfiguration` (primary and secondary text, image identifier, image size and spacing, margins, per-state text colours) and `layout(bounds, text_h, secondary_h)` returning the image, text and secondary text rectangles |
| `springboard/ui/src/reserved_region.rs` | `ReservedRegionKind` (keyboard, window controls, system), `ReservedRegion` (identifier, frame including margins, margins, active flag, `occupied()`), `ReservedRegions` (declare, remove, set_active, query, region_at, free_area) |

Shared rules:

- State priority is disabled, then selected, then highlighted, then normal. A state that has no value of its own
  uses the next state in that order, and finally `normal`.
- A list cell lays its parts out in its bounds: the image sits at the leading edge, centred vertically; the text column
  takes the rest of the width; the two text lines form one block that is centred vertically. A block taller than the
  area starts at its top edge.
- `free_area` cuts a region only when it spans the full width or the full height of the remaining area. A region that
  crosses a corner or the middle is ignored, because the remaining space would not be a rectangle. Regions apply in
  declaration order.
- Inactive regions do not reduce the free area and are not returned by `region_at`.

## How an app uses it

An app describes the cell and leaves the placement to the toolkit:

```rust
use bifrost_app::controls::list_content::ListContentConfiguration;
use bifrost_app::controls::background_configuration::{BackgroundConfiguration, ConfigurationState};

let mut content = ListContentConfiguration::subtitle("Mail", "3 unread");
content.image = Some("mail-icon".to_string());
content.secondary_color.selected = Some([255, 255, 255, 255]);

let background = BackgroundConfiguration::filled(
    [0, 0, 0, 0],       // normal: transparent
    [0, 0, 0, 20],      // highlighted: a light press shade
    [10, 132, 255, 255] // selected: the accent
);

let state = ConfigurationState { selected: true, ..Default::default() };
let parts = content.layout(cell_bounds, measured_title_h, measured_detail_h);
let plate = background.resolve(cell_bounds, state);
```

A view declares the system UI it must leave clear:

```rust
use bifrost_app::controls::reserved_region::{ReservedRegion, ReservedRegionKind, ReservedRegions};

let mut regions = ReservedRegions::new();
regions.declare(
    ReservedRegion::new("keyboard", ReservedRegionKind::Keyboard, keyboard_frame)
        .with_active(keyboard_raised),
);
let content_area = regions.free_area(view_bounds);
```

On a desktop with a hardware keyboard, the keyboard region is declared inactive and never raised, so the window area
is not reduced. Window controls declare their region as a `WindowControls` region so a pointer event on them can be
routed with `region_at`. A desktop does not need the on-screen keyboard, but a pointer or touch event still goes through
the same routing.

## Not done yet

- Readable content guide (a layout guide that is narrower than the view for long text).
- The list cell and view paint code does not read these configurations yet. The toolkit paints the parts and the
  background once the cell and view adoption lands.
- The keyboard and window controls do not declare their regions in the view tree yet. The shell has to call
  `declare` when it raises the keyboard or lays out the window controls.
- No test runs on the tablet; the lead runs the unit tests and the tablet check after the merge.
