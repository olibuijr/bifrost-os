# Search (Spotlight-style) polish: parity slice `spotlight`

Round 75. Extends the existing shell Search card (`springboard/src/shell/quicksearch.rs`, unit conversion in
`quicksearch_math.rs`). No new app, no new crate. Nothing was built or run: the code was checked by reading and
the lead runs the tests.

Reference evidence: none was used for behaviour. The macOS 27 reference extracts under
`~/Projects/macos-ref/extract/` were not consulted in this round (the brief's extract list was not read), so the
behaviour below follows the existing Search card and the owner's list, not measured reference captures.

## What is done

| Feature | BifrOSt path | State |
|---|---|---|
| Inline calculator | `src/shell/quicksearch_math.rs` `calc`, group "Calculator" | Already present; tests added (precedence, parentheses, `^` right-associative, `%`, infinite results give no answer) |
| Inline unit conversion | `quicksearch_math.rs` `UNITS`, `convert` | Extended: volume (ml, l, tsp, tbsp, fl oz, cup, gal), speed (m/s, km/h, mph, knots), data (B, kB, MB, GB, TB, KiB, MiB, GiB). Length, weight, temperature were already there. Tests for each kind and for mixed kinds |
| Open containing folder | `quicksearch.rs` `Hit::Reveal`, `file_note_groups` ("Show in Folder" group for the top file match); `files/mod.rs` `Files::show_folder` | New. Return on the row opens Files with the file's folder pushed on the location list |
| Quick Look with Space | `quicksearch.rs` `Search::key`: Space on an arrow-selected `Hit::File` opens the preview in Files | New. Typing a space still types one when no file row is selected; typing clears the arrow selection, so Space only previews a row the arrows chose |
| App actions (`[[intent]]`) | `quicksearch.rs` `shortcut_items`, group "Shortcuts" | Already present, unchanged |
| Contacts | `quicksearch.rs` `results_folded`, group "Contacts" | Already present, unchanged |
| Recent searches | `quicksearch.rs` `note_search`, `searches`, shown under the empty field | Already present, unchanged |
| Documents from app indexes | `quicksearch.rs` `indexed_groups`, `src/shell/spotindex.rs` | Already present, unchanged. This is the Core Search index that apps feed through `CSSearchableIndex`; it is the closest thing to a content index in the repo |

## What is still open

- **Dictionary definitions.** Not done. The StarDict reader is in the Dictionary app crate
  (`apps/dictionary-desktop/src/dict.rs`), and the shell cannot import an app crate. An inline definition needs
  the reader moved to a shared crate (`bifrost-ui` or a small new crate) and the Dictionary app migrated to it.
  Opening the Dictionary app at a word also needs a request channel, which it does not have (no argv or handoff
  file is read by that app).
- **File content index.** Not done. The repo has no `mdfind`-style content index. Search only matches file names
  (`files/fs.rs` `recent_in`, 2000 newest files on a worker thread). A content group needs an indexer that owns
  extracted text, which is a larger job than this round.
- **Calendar events and Mail results.** Not done. No shell-side source exists for either. The Calendar and Mail
  apps do not feed the Core Search index (only Books, Contacts, Files and Notes do in `apps/*/src`), so the
  Documents group cannot show them yet. Each app would need to index its items the way Notes does.
- **Quick Look as a true overlay.** Space opens the file in Files' preview page and drops the card, as Return
  already did for files. A preview that keeps the card up is not done.

## Shared-file edits

- `springboard/src/files/mod.rs`: one new method, `Files::show_folder`, added after `preview_entry`. Nothing else
  changed in that file.

## Midget checks still needed

- Type a file name (for example a PDF in Documents), arrow down to the file row, press Space: Files should open on
  that file's preview, with the search card gone.
- Type a file name and tap "Show <name> in Folder": Files should open on the folder, with the location list under
  it so Back returns to the locations.
- Type `100 km/h to mph`, `1 gb to mb`, `2 l to ml`: the answer row shows `= ...` as the top group.
- Dark and Light: the new "Show in Folder" group header and row use the same colours as the Files group.
