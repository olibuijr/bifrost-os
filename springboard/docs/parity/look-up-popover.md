# Look Up popover (slug: look-up-popover)

Round 75. Owner list: draw the Look Up popover (the `bifrost_ui::look_up` model existed, drawing did not): the
dictionary definition, the quick search results and an Open in Dictionary action. Ctrl+Cmd+D and the Services menu's
Look Up open it; Escape closes it.

## Evidence used

- `~/Projects/macos-ref/extract/ref27/System/Applications/Dictionary.app/Contents/Resources/en.lproj/ServicesMenu.strings`:
  the service's menu label is "Look Up in Dictionary" (the label the Services entry keeps).
- `Dictionary.app/Contents/Info.plist`: the `NSServices` entry (`doLookupService`, text send type), as in
  `docs/parity/services-lookup.md`.
- `Dictionary.app/Contents/Resources/`: the nibs `DescriptionView.nib`, `DictionaryWindow.nib` and `ScopeBarView.nib`
  exist in the reference. They were not decoded with REA this round, so the popover's layout, its spacing and its
  section titles are BifrOSt's own design, not measured from the reference. Needs a decode round to check.

## Rows

### Look Up popover (dictionary, quick search, actions)

- API: pasteboard service `doLookupService` as the model of what the service takes (`NSServices`).
- BifrOSt path:
  - `springboard/ui/src/look_up.rs`: `LookUp` (term, definition, search hits), `Row` (Heading, Text, Search,
    OpenInDictionary), `rows()`. The rows are Dictionary (the definition, cut to 320 characters, or "No definition in
    the installed dictionaries."), Search (up to five quick search titles), then the Search and Open in Dictionary
    actions.
  - `springboard/ui/src/stardict.rs`: the StarDict reader, moved out of the Dictionary app so the shell reads the same
    dictionaries (`load_library`, `SYSTEM_DICTIONARIES`, `WORD_LIST`). The Dictionary app now imports it.
  - `springboard/src/shell/look_up_popover.rs`: `Popover` (layout, `tap`, `draw`), `content(term)` (the first
    definition from the library, the titles of `quicksearch::results`), `title_of`, and the Shell glue
    (`open_look_up`, `look_up_event`, `look_up_escape`, `draw_look_up`).
  - Shell wiring: `springboard/src/shell/mod.rs` (the `look_up` field, the event hook before `modal_event`, the draw and
    damage lines, the two quiet-frame checks), `springboard/src/shell/hwkeys.rs` (`Shortcut::Back` closes the popover
    first).
- Done: Ctrl+Cmd+D (`look_up_selection` -> `SelectionUse::LookUpPopover` -> `open_look_up`, in
  `springboard/src/shell/services_route.rs`) opens the popover centred under the menu bar. Services > Look Up in
  Dictionary does not open the popover: like the reference, it opens the Dictionary app on the term (see below). The definition is the first installed dictionary's answer. Tapping Search closes it and opens
  Quick Search with the term; tapping Open in Dictionary closes it and runs the Dictionary app's `look-up` intent. A
  tap outside, or Escape, closes it. Unit tests cover the rows, the cut, tap mapping, the screen clamp, wrapping and
  quick search titles.
- Needs Midget: the popover's look and size; the first Look Up on a machine with the system dictionaries (the lookup
  reads each `.ifo` and `.idx` file on each Look Up, not cached); Escape closing it when the desktop has focus.

### Services > Look Up in Dictionary (anchor)

- BifrOSt path: the Services item routes to `Route::Intent { app: DICTIONARY_APP, intent: "look-up", param: "word" }`
  in `springboard/ui/src/services_menu.rs`, which `Shell::run_intent` (`springboard/src/shell/services_route.rs`)
  runs on the Dictionary app. The services model has no Look Up route of its own any more; the popover is reached only
  through the chord (`SelectionUse::LookUpPopover`). Its Open in Dictionary row runs the same intent.
- Gap: the popover opens at the top centre, not at the selection. The shell does not read the focused app's selection
  rectangle (the Services notes in `services-lookup.md` have the same gap), so "near the selection" is not met yet.

### Gaps not closed here

- Quick search hits come from the desktop's own sources (apps, Settings, World Clock, Files, contacts). Deildu titles
  are not passed in, so a Deildu title does not show in the popover.
- The Dictionary lookup is synchronous on the UI thread. A large system dictionary may cause a visible pause on the
  first Look Up. Move it to a worker thread if the lab PC shows a hitch.
- Three-finger tap (listed in `services-lookup.md`) is not part of this slice.

## Shared-file edits (merge carefully)

- `springboard/ui/src/lib.rs`: one `pub mod stardict;` line.
- `springboard/src/shell/mod.rs`: `mod look_up_popover;`, the `look_up` field and its `None` initialiser, the
  `look_up_event` call before `modal_event`, the draw call after `draw_share`, the damage condition, and
  `self.look_up.is_none()` in the two quiet-frame checks (`terminal_key`, `settings_key`).
- `springboard/src/shell/hwkeys.rs`: one `Shortcut::Back if self.look_up_escape()` arm before the character viewer's.
- `springboard/src/shell/services_route.rs`: `run_service` now calls `run_intent`, `search_for` and `open_look_up`.

## Migrations and deletions

- Deleted: `springboard/apps/dictionary-desktop/src/dict.rs` (moved to `ui/src/stardict.rs`), and the app's own
  `SYSTEM_DICTIONARIES`, `WORD_LIST`, `dictionary_dirs` and `load_library` (now shared).
- Changed: this slice first replaced `services_menu::Route::Intent` for Look Up with a `Route::LookUp`; a later fix
  (E2E11) removed `Route::LookUp` again, so Look Up routes to the Dictionary intent and the chord alone opens the
  popover. The Services test `look_up_sends_the_first_line_as_the_word` is now
  `look_up_takes_the_first_line_as_the_term`.
- App bump: `bifrost-dictionary` 0.1.0 -> 0.1.1 (`Cargo.toml`, `manifest.toml`); it gained the `bifrost-ui` dependency.
  `Cargo.lock` needs the new edge (the lead's build updates it).

## Not built in this round

- Nothing was compiled or tested (the lead builds). The unit tests are written but unrun.
- No parity matrix rows were edited (`docs/parity-matrix/`), and none were verified as closed.
