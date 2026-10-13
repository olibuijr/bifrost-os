# Parity: Font Book and Console (round 75, slug fontbook-console)

Apps: `springboard/apps/fontbook-desktop` (0.2.0) and `springboard/apps/console-desktop` (1.5.0).
Matrix rows: `docs/parity-matrix/os/apps/d-f.json` ("Font Book", partial) and `a-c.json` ("Console", partial),
`docs/parity-matrix/data/oslog.json` (Console rows). The matrix is not edited here; see the closures below.

Evidence: the code and tests in this worktree, and the reference names in the macOS 27 extract index
(`~/Projects/macos-ref/extract/ref27-index.tsv`: `Font Book.app/Contents/Info.plist`, `Console.app/Contents/Info.plist`,
and the Console `.nib` resources). The Info.plist and nib files were not measured with REA in this round. Behaviour
is taken from the existing app code and its own notes. No REA evidence ids are recorded.

Not built or run in this round (the lead builds and tests). Every item below still needs a build, the unit tests and a
Midget check.

## Font Book

| Area | Apple surface (names only) | BifrOSt path | Status |
|---|---|---|---|
| Validate fonts on install | Font Book's validation of a font before it is enabled | `fontbook-desktop/src/validate.rs` (`check`, `check_file`), used by `model::install` | Done in code; needs Midget check with a broken file |
| Repertoire (characters, blocks) | Character repertoire of a font face | `validate.rs` (`repertoire`, `format4`, `format12`, `BLOCKS`), shown by `model::repertoire_rows` in the Info panel | Done in code; coverage is by Unicode block name only, no glyph grid |
| Remove font | Remove a font the user installed | `model::uninstall` wired to the Remove Font button (`main.rs`, `touch_down`, user-installed files only) | Done in code; needs Midget check |
| Collections (create, membership) | Font collections in the sidebar | `model::Library::create_collection`, `add_to_collection`, `remove_from_collection`; `main.rs` New Collection row and membership rows | Done in code; no rename or delete yet |
| Preview text | Editable sample text in the preview | Not done. The preview still draws `model::PREVIEW_TEXT` only | Gap |
| Per-style listing | Faces of a family listed separately | Not done. Still one face per family | Gap |

Notes:
- The validator reads the tables by bounds, so a truncated file reports each table that runs past the end and the
  tables still missing. It does not decode outlines. A font that passes is not guaranteed to draw; the preview still
  uses the renderer's own `Font::parse` as the final check.
- Font collections (`.ttc`) are refused on install with a reason. Their faces are not listed yet.

## Console

| Area | Apple surface (names only) | BifrOSt path | Status |
|---|---|---|---|
| Live journal stream | Live messages from the system log | `source.rs` (`Journal::follow`), `console.rs` (`Start`/`Pause Streaming`) | Existing; unchanged |
| Filters by process, subsystem, level | Search tokens and level toggles | `model.rs` (`Query::parse`: `process:`, `subsystem:`, `category:`, `message:`, `level:`), toolbar Info and Debug toggles, Log menu Show Only This Subsystem | Existing; unchanged |
| Crash reports list | Crash reports from the system's crash service | `source.rs` (`Coredumps`), `console.rs` View::Crashes, `model.rs` (`parse_coredump_list`) | Existing; unchanged |
| Search and saved searches | Search field, saved searches | `console.rs` (`keep_search`, `drop_search`, sidebar) | Existing; unchanged |
| Save selection to a file | Save a selection of messages | `export.rs` (`text`, `file_name`, `save`), `console.rs` (`save_messages`), Log menu "Save Messages..." (`main.rs`) | Done in code; a save never overwrites; needs Midget check |

Notes:
- Save Messages writes the selected message, or every message the window shows when none is selected. The file goes
  to `~/Documents/Console/Console-<unix seconds>.log`. The path or the error shows in the status line.
- The Console had no multi-select; the selection is one message, so a save of several rows is the visible list.

## Shared-file edits

None. The changes are inside the two app directories, the root `springboard/Cargo.lock` (two version lines for
`bifrost-console` and `bifrost-fontbook`), and this document.

## Migrations and deletions

- `model::Library` dead-code attributes removed for the functions that are now wired (collections, `clean`,
  `uninstall`). `disabled_names` and `delete_collection` are now compiled only under `cfg(test)`.
- `main.rs` (Font Book) rewritten around `touch_down`; the old inline hit test in `event` is gone.
- No superseded code was deleted from the Console.

## Matrix entries closed by this round

None closed on the matrix yet. The lead should move the Font Book and Console rows to `partial` with the notes above,
and close them only after the Midget checks.
