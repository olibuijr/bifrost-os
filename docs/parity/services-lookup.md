# Services menu and Look Up (slug services-lookup)

Round 75. Branch `parity/services-lookup`, based on `wave-base` (3355822c).

## Evidence used

- Reference Info.plist of Dictionary (`~/Projects/macos-ref/extract/ref27/System/Applications/Dictionary.app/Contents/Info.plist`),
  key `NSServices`: one service, menu item "Look Up in Dictionary", message `doLookupService`, send type plain text.
- Reference index rows for ServicesMenu.strings (`~/Projects/macos-ref/extract/ref27-index.tsv`).
- Stickies reference Info.plist declares `NSServices` with a "Make Sticky" entry. The brief's title is "Make New Sticky Note".
  Notes declares no services in the reference.
- Code read: `springboard/src/shell/sharing.rs` (share sheet delivery), `springboard/src/shell/desktop_bar.rs` (Edit menu,
  context menu dispatch), `springboard/src/shell/hwkeys.rs` (chords), `springboard/src/shell/quicksearch.rs`,
  `springboard/src/shell/workspace.rs` (route_url), `springboard/sdk/src/definitions.rs` (dictionary lookup model, SDK only).
- REA: not run in this round.

## Rows

Matrix rows are not edited (the lead closes them). Rows that this slice touches:

- `core-services.json`, "Dictionary lookup" (DCSCopyTextDefinition, DCSGetTermRangeInString, DCSDictionary): still partial.
  The Look Up route into the Dictionary app exists in the shell. No definition API for apps yet.
- No matrix row names the Services menu. The lead decides whether to add one.

## What is done

| Feature | Where | Status |
|---|---|---|
| Services entries and titles | `springboard/src/shell/services_menu.rs` (`Service`, `title`, `id`, `from_id`) | model, tested |
| Which services apply to a text, link or file | `for_text`, `for_payloads`, `is_web_link` | model, tested |
| Target of each service | `target` -> `Target::{Web, App, Search, Files}` | model, tested |
| Services rows at the foot of Edit | `desktop_bar.rs` `services_rows`, Edit arm; `Action::Service` | wired, not built |
| Route to the target | `Shell::run_service`: Web -> `workspace_open_url`, App -> `deliver_text`, Search -> `search_for` | wired, not built |
| Text to an app | `deliver_text` uses `sharing::text_url` and `ToApp::Open`, as the share sheet does. The share delivery now uses the shared `open_for_inbound` | wired, not built |
| Dictionary takes the word | `springboard/apps/dictionary-desktop` `Input::Open` -> `look_up` | wired, app 0.2.0 |
| Notes takes a new note | `springboard/apps/notes-desktop` `Input::Open` -> `import_note` | wired, app 1.15.0 |
| Stickies takes a new sticky | `springboard/apps/stickies-desktop` `Input::Open` -> `new_note_with` | wired, app 0.3.0 |
| Search With quick search | `QuickSearch::set_query` (quicksearch.rs) runs the search for the text | wired, not built |
| Look Up popover | `look_up_items` -> desktop context menu (`look_up_popover`); Ctrl+Cmd+D in `hw_chord` | wired on the desktop, not built |
| Look Up popover quick search results | not shown; the popover lists the term and three services | open |
| Show in Files | `Target::Files` modelled; the shell never has a file on the clipboard | model only |
| Force click, three-finger tap | no pressure or multi-finger events reach the shell | not done |

## Still needs Midget checking

- The Services rows appear in Edit with clipboard text, and each entry opens its app.
- Dictionary shows the word after Look Up. Notes and Stickies create the note with the text.
- Ctrl+Cmd+D opens the popover on the desktop when the clipboard holds a term. A click outside or Escape closes it.
- Search With quick search fills the field and shows results.
- Open URL opens the default browser on a web link.

## Limits

- The selection is the shell clipboard. SDK apps fill it with Copy. Apps do not publish their selection, so Services act on
  the last copied text, not the live selection.
- The popover shows no definition text. The dictionary app shows the definition after the hand-off.
- No file selection reaches the Services menu, so Show in Files is not reachable from the shell yet.
