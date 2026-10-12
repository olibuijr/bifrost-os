# Wave 3, theme-08: preference pane class (NSPreferencePane)

Theme: "Preference pane class (NSPreferencePane)", size 28 rows across 20 frameworks. This wave closes three
rows of the Preference Panes framework that the wave 2 pane model did not cover, and advances the pane class row.
The other theme rows (certificate panels, keychain panels, print plug-ins, screen saver sheets, ScreenCaptureKit
picker and similar) have no counterpart in BifrOSt today; they stay gaps and are not claimed here.

## What was built

All work extends the existing pure model `springboard/ui/src/pane.rs` (wave 2). No new module was added, and
`lib.rs` is unchanged. Std only, no unsafe, no new crates.

- `PaneHooks::load_main_view` and `PaneHooks::main_view_did_load`: the host calls `load_main_view` once, between
  `will_select` and `did_select`, on the first selection of a pane. The returned view handle is stored and
  `main_view_did_load` runs after it. A `None` result means the next selection asks again. `PaneHost::main_view`
  reads the stored handle.
- `PaneHost::key_loop(name, views)`: the keyboard focus chain of a pane over the view handles it owns. It starts at
  `initial_key_view`, walks forward and wraps, and stops at `last_key_view` (both inclusive). A missing or unknown
  initial view starts at the first view, and a missing or unknown last view ends at the last one.
- `parse_manifest(text) -> Result<PaneSpec, ManifestError>`: a text manifest of `key = value` lines (`name`,
  `title`, `auto_save_text_fields`, `initial_key_view`, `last_key_view`, repeatable `help = Title | anchor`). It
  is the BifrOSt counterpart of a pane bundle's property list and its help menu keys. `ManifestError` reports the
  line number of each bad line or value.
- Tests: `main_view_loads_once_between_will_select_and_did_select`, `key_loop_walks_from_initial_to_last_and_wraps`,
  `key_loop_falls_back_to_first_and_last_views`, `manifest_reads_every_key`, `manifest_reports_the_line_of_each_error`.
  The wave 2 tests (lifecycle order, veto, notices, auto save, Help menu) are unchanged; they were not run in this wave.

## Official API mirrored

From the official Preference Panes pages (DocC and rendered page):

- `NSPreferencePane`: `loadMainView()`, `assignMainView()`, `mainViewDidLoad()`, `mainView`, `willSelect()`,
  `didSelect()`, `willUnselect()`, `didUnselect()`, `shouldUnselect`, `firstKeyView`, `initialKeyView`,
  `lastKeyView`, `autoSaveTextFields`.
- Help menu keys: `NSPrefPaneHelpMenuInfoPListKey`, `NSPrefPaneHelpMenuTitleKey`, `NSPrefPaneHelpMenuAnchorKey`.

Not mirrored: the bundle loading itself (`init(bundle:)`, `bundle`, `mainNibName`), nib loading, and the
`assignMainView()` lookup. BifrOSt builds pages from code and has no nib, so the pane loads its view through a
hook, not from a file.

## REA evidence

- `ev_0f4a23642261b0d646f88ae02b62954a6fcef14cb006a2b4223ec7a54e1e3ba1`: rendered page
  https://developer.apple.com/documentation/preferencepanes (title "Preference Panes").

## Matrix rows closed (status partial)

| framework slug | feature (exact text in the matrix) | status | BifrOSt path |
|---|---|---|---|
| preference-panes | Preference pane class (NSPreferencePane) | partial | springboard/ui/src/pane.rs (PaneHooks, PaneSpec, PaneHost) |
| preference-panes | Loading the main view | partial | springboard/ui/src/pane.rs (PaneHooks::load_main_view, PaneHooks::main_view_did_load, PaneHost::main_view) |
| preference-panes | Pane bundle format and Info.plist keys | partial | springboard/ui/src/pane.rs (parse_manifest, ManifestError) |
| preference-panes | Keyboard focus chain in a pane | partial | springboard/ui/src/pane.rs (PaneHost::key_loop) |

## Limits

- The pane host is not wired into the Settings app (`springboard/src/shell/settings.rs`). Settings still builds
  its pages from its own page enum, and no external manifest is loaded from disk.
- The manifest is a BifrOSt text format, not the reference property list. It cannot read a property list file.
- `key_loop` orders the views it is given; it does not discover the views of a pane's layout.
- Not built, not run on the VM or Midget: no build and no tests were run in this wave, by rule.
