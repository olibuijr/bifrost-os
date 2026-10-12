# Wave 4, theme 11: print dialog plug-ins and panels

Theme: "Print dialog plug-ins (PDEPlugIn, PDEPanel)" (docs/parity-matrix/themes.json, theme-11).

## What was built

- `springboard/sdk/src/print_panel.rs` (`bifrost_app::print_panel`), a pure model of print dialog plug-ins:
  - `PlugIn` is a named bundle of `PrintPanel`s, the unit an app registers in `PlugIns`.
  - `PrintPanel` has a `Placement` (`PrintDialog` or `PageLayout`), so one model covers the print panel's accessory
    views and the page layout panel's accessory views.
  - `PanelOption` is typed: `choice`, `toggle` and `number` constructors validate the key, the choice list, the range and
    the default. `lp_value()` gives the CUPS value and `display()` gives the text a panel shows.
  - `PlugIns::register` keeps panel ids and option keys unique across all plug-ins, rejects keys that the print info already
    writes (`copies`, `media`, `landscape`, `page-top`, and so on), and refuses a plug-in whole when any check fails.
  - `PlugIns::set` checks the key, the kind and the range, and changes nothing on error.
  - `PlugIns::lp_options` renders every option as `-o key=value` in registration and panel order.
- `springboard/ui/src/print.rs`: `PrintOperation` gains `options` and `with_options(..)`. `lp_args` appends them after the
  print info options and before `--`. Existing argument order is unchanged when no options are set.
- `springboard/sdk/src/lib.rs`: one `pub mod print_panel;` line.

Not built: the shell-drawn print dialog and page setup sheet (no proto tags, no drawing). Plug-in code that runs inside
another process (the `PDEPlugIn` bundle loading and host callback protocol) is not built; a plug-in here is registered by
the app that owns it. Apps show the panels with their own kit controls.

## Official API names mirrored

- `NSPrintPanel`: accessory controllers (`addAccessoryController`, `removeAccessoryController`,
  `accessoryControllers`) and the printing info the panel runs with (`printInfo`).
- `NSPageLayout`: accessory controllers for the page setup panel, placed by `Placement::PageLayout`.
- `PDEPlugIn`: a plug-in bundle that supplies panels for a print type (`pdePanels(forType:withHostInfo:)`); the model
  is `PlugIn` with its `PrintPanel`s.

## REA evidence (rendered official pages)

| Page | REA evidence id |
|---|---|
| https://developer.apple.com/documentation/appkit/nsprintpanel | ev_9fa8bca031cb5b25f8409969c1f5416b2078b8039a79c03eaa149fd3c672877e |
| https://developer.apple.com/documentation/appkit/nspagelayout | ev_030bef994803945ddd26567a13dce994a0399924f25420989458e147ed0dc7c9 |
| https://developer.apple.com/documentation/applicationservices/pdeplugin | ev_62f2e8b050db15e266d4f7792af05839160b9b871b9c20f385ce157e7f133d9e |

## Matrix rows closed (status partial)

| Framework slug | Feature (as in data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| application-services | Print dialog plug-ins (PDEPlugIn, PDEPanel) | partial | springboard/sdk/src/print_panel.rs |
| appkit-graphics-printing | Printing: print panel, page layout, accessory views | partial | springboard/sdk/src/print_panel.rs |

Partial, not parity: no print dialog or page setup sheet is drawn by the shell, and plug-ins are not loaded from bundles.

## Tests

`print_panel.rs` has seven host tests: registration refusals (duplicate id, duplicate key, reserved key, bad plug-in id),
constructor range checks, `set` key/kind/range checks, `lp_options` ordering, placement filtering, display text, and the
`lp` argument tail with the document after `--`. Per the wave rules nothing was built or run; the expected values were
worked out by hand from the rules the code implements.

## Owner rule

`python3 tools/check-names.py` exits 0 with the new and changed files.
