# Services menu and Look Up (slug: services-lookup)

Round 75. Owner list: the App menu's Services submenu (Look Up in Dictionary, New Note With Selection, Make New Sticky
Note, Open URL, Search With Quick Search, Show in Files), the shell routing of the selection to the target app, and a
Look Up popover with dictionary and quick search results on Ctrl+Cmd+D and three-finger tap.

## Evidence used

- `~/Projects/macos-ref/extract/ref27/System/Applications/Dictionary.app/Contents/Info.plist`: `NSServices` entry,
  menu title "Look Up in Dictionary", `NSMessage` `doLookupService`, `NSSendTypes` text.
- `~/Projects/macos-ref/extract/ref27/System/Applications/Stickies.app/Contents/Info.plist`: `NSServices` entry,
  menu title "Make Sticky", `NSMessage` `makeStickyFromTextService`, `NSSendTypes` text. The menu title used here is the
  owner's "Make New Sticky Note".
- `System/Applications/Notes.app/Contents/Info.plist` declares no services (its path is in the reference index).
- Not measured with REA in this round: the popover's layout, and the Services submenu's exact grouping in the App menu.
  The App menu flattens the submenu into a labelled group (the shell's menus have no nested rows).

## Rows

### Services menu (App menu > Services)

- API: pasteboard services (`NSServices`, `NSSendTypes`, `NSMessage`) as the model of what each service takes.
- BifrOSt path: `springboard/ui/src/services_menu.rs` (`Service`, `Selection`, `menu_for`, `route`, `web_url`,
  `file_path_of`); `springboard/src/shell/services_route.rs` (`selection`, `menu_entries`, `run_service`);
  `springboard/src/shell/desktop_bar.rs` (App menu rows, `Action::Service`).
- Done: the App menu lists only the services that take the current selection. The selection is the pasteboard's
  file URLs (paths), else its plain text, else the plain clipboard text. Unit tests cover the menu, routes and file URLs.
- Still needs Midget: the selection source. Wayland has no primary selection in the shell, so the selection is what
  was last copied (Cmd+C). A select-then-menu flow needs the focused app's selection, which the shell does not read yet.

### Look Up in Dictionary

- BifrOSt path: route `Intent { app: DICTIONARY_APP, intent: "look-up", param: "word" }`, run by
  `shell/intents.rs::run_with` over the app's control socket (`intent.look-up` with `{"word": ...}`).
- Done: the term is the first line of the selection, trimmed, up to 64 characters (`ui/src/look_up.rs::term_of`).
- Needs Midget: the Dictionary app opens and answers the intent (it returns the definition as the reply's dialog).
  The reply is not yet shown anywhere except the Dictionary app itself.

### New Note With Selection

- BifrOSt path: route `Intent { app: NOTES_APP, intent: "create-note", param: "text" }`. The Notes manifest has
  `create-note` with `text:string?`.
- Needs Midget: the Notes app creates the note with the selection.

### Make New Sticky Note

- BifrOSt path: route `Intent { app: STICKIES_APP, intent: "create-sticky", param: "text" }`.
- Gap: Stickies declares no intents, so the call fails with "no intent" until the Stickies app gets a `create-sticky`
  intent (app change: manifest, handler, version bump). Not done in this round.

### Open URL

- BifrOSt path: `Route::OpenUrl` for one http or https address with no spaces, opened with `desktop_open_url`.
- Done in the model and wired. Needs Midget: the browser opens the page.

### Search With Quick Search

- BifrOSt path: `Route::Search`, which calls `quicksearch_show` and sets the field text with `TextField::set_text`.
- Needs Midget: the text appears in the field and the results update.

### Show in Files

- BifrOSt path: `Route::Reveal` takes the first file path; the shell shows the folder itself when it is a folder, else
  the folder that holds the file (`reveal_folder`), through `desktop_open_folder` for `is.olibuijr.files.desktop`.
- Needs Midget: Files opens at the folder.

### Ctrl+Cmd+D (Look Up)

- BifrOSt path: `springboard/src/shell/hwkeys.rs` `Shortcut::LookUp` (KEY_D with Ctrl and Cmd, no Alt or Shift),
  dispatched to `run_service(Service::LookUp)`. The test is in `hwkeys.rs` (chord test next to the reader chords).
- Done: the key is bound and runs Look Up in Dictionary on the selection.

### Look Up popover (dictionary and quick search)

- BifrOSt path: `springboard/ui/src/look_up.rs` (`LookUp`, `Row`, `rows`, `HITS_SHOWN` = 5). The rows are a Dictionary
  section (the definition, or "No definition in the installed dictionaries."), a Search section with up to five quick
  search hits, and a Search row.
- Gap: the model is not drawn and the shell does not fill it yet. Ctrl+Cmd+D shows the definition in the Dictionary app,
  not in a popover. The drawing needs the shell's popup surface and the quick search results for the term.

### Three-finger tap

- Gap: the shell's touch input carries one point per event (`tap`, `down`, `move`, `up`), with no finger count. Adding a
  three-finger tap needs the input path to report the count, which is outside this slice. Not done.

## Shared-file edits

- `springboard/ui/src/lib.rs`: two `pub mod` lines (`services_menu`, `look_up`).
- `springboard/src/shell/mod.rs`: one `mod services_route;` line.
- `springboard/src/shell/intents.rs`: `request_line_with`, `run_with`, `json_string`; `run_at` takes a request line.
- `springboard/src/shell/desktop_bar.rs`: `Action::Service(u8)`, the App menu Services group, the dispatch arm.
- `springboard/src/shell/hwkeys.rs`: `Shortcut::LookUp`, `KEY_D`, the decode arm, the dispatch arm, one chord test.

## Not built in this round

- Nothing was compiled or tested (the lead builds). The unit tests are written but unrun.
- No parity matrix rows were edited (`docs/parity-matrix/`), and none were verified as closed.
