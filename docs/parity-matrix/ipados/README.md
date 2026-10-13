# Tablet platform parity data

Assessment of the tablet platform's UI framework (slug prefix `uikit-`) against BifrOSt, for the computer and tablet
convergence work. It follows `docs/parity-matrix/agent-brief.md`; official facts come from
`python3 -I tools/parity/docc.py uikit/<section>`.

`data/<slug>.json` has the same keys as `docs/parity-matrix/data/*.json`: `framework`, `doc`, `macos_since` (empty: the
framework is not part of the desktop SDK), `status`, `bifrost`, `summary`, `rea_evidence`, `features[]`
(`feature`, `apple`, `bifrost`, `status`, `note`) and `gaps[]`. Statuses are `parity`, `partial`, `os`, `gap`, `n/a`.
A row's `bifrost` is `none` or lists repo paths that exist; a `none` row is `gap` or `n/a`.

| File | UIKit topic sections |
|---|---|
| `uikit-views-controls` | Views and controls |
| `uikit-touches-gestures` | Touches, presses, and gestures |
| `uikit-text-input-keyboards` | Keyboards and input |
| `uikit-text-display-textkit` | Text display and fonts, TextKit, Writing Tools, Handwriting recognition |
| `uikit-app-scene-lifecycle` | App and environment, App extensions, Resource management |
| `uikit-view-controllers` | View controllers |
| `uikit-collections-tables` | Collection views, Table views |
| `uikit-drag-drop` | Drag and drop |
| `uikit-pointer-menus-focus` | Menus and shortcuts, Pointer interactions, Focus-based navigation, Pencil interactions |
| `uikit-windows-multitasking` | Windows and screens, multitasking |
| `uikit-appearance-animation-layout` | View layout, Appearance customization, Animation and haptics |
| `uikit-accessibility` | Accessibility for UIKit |
| `uikit-graphics-printing-pencil` | Images and PDF, Drawing, Printing |

The `uikit-` prefix maps to the framework title `UIKit` in `PARTS` (`tools/parity/parity.py`), so each part counts as a
UIKit assessment: `check --platform ipados` does not list the UIKit index row as not assessed.

## Status of the rows that the convergence work touched

The touch, form factor, rotation, text input, gesture recognizer, login keyboard and desk keyboard rows now cite the arm code
(`springboard/wm/src/touch.rs`, `springboard/src/bare/wl_touch.rs`, `springboard/ui/src/form_factor.rs`, `springboard/wm/src/transform.rs`,
`springboard/ui/src/recognizer/`, `springboard/src/shell/login.rs`, `springboard/src/shell/desk.rs`). Their notes say "not yet verified on the
tablet": the status is `partial` until a tablet run (and, for the matrix, a Midget check) confirms the behaviour.
