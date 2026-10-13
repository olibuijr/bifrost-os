# Dictionary and Stickies (round 75, slice dictionary-stickies)

Two apps: `apps/dictionary-desktop` (new, crate `bifrost-dictionary`) and `apps/stickies-desktop` (crate
`bifrost-stickies`, version 0.2.0). Both draw with the shell's kit (`bifrost_app::kit`) and keep their data in the
app's data directory.

## Evidence used (reference files, names and behaviour only)

- `~/Projects/macos-ref/extract/ref27/System/Applications/Dictionary.app/Contents/Info.plist`: the `NSServices`
  entry "Look Up in Dictionary" (message `doLookupService`, send type `NSStringPboardType`), the `x-dictionary`
  URL scheme, the reference category, and the `DefaultFoundCountCap` setting (1000).
- `.../Dictionary.app/Contents/Resources/en.lproj/ServicesMenu.strings`: the service's menu title.
- `~/Projects/macos-ref/extract/ref27/System/Applications/Stickies.app/Contents/Info.plist`: the document type
  (one note per document), the `NSServices` entries ("Make Sticky", key Y) and the reference category.
- Stickies' note resources (`en.lproj/*.rtfd`) were not copied; the colours and the layout come from the design
  rules in `docs/bifrost-ui.md` and the kit tokens.
- Matrix rows (names only, not edited): `docs/parity-matrix/os/apps/d-f.json` "Dictionary" (gap),
  `docs/parity-matrix/os/apps/s-s.json` "Stickies" (gap, note "no sticky notes app yet", now stale).

## Dictionary

| API / behaviour | BifrOSt path | Done | Still needs Midget checking |
|---|---|---|---|
| Dictionary lookup (StarDict `.ifo`/`.idx`/`.dict`) | `apps/dictionary-desktop/src/dict.rs` (`Dictionary`, `Library`, `parse_ifo`, `parse_idx`, `strip_tags`) | Parser, sorted index, case-insensitive find, prefix suggestions, HTML definitions, 32- and 64-bit offsets; unit tests on synthetic files | Real dictionary files from the device (none are installed on the host) |
| Search field and suggestions | `apps/dictionary-desktop/src/main.rs` (`edited`, `lines`, `key`) | Typing filters suggestions; Return looks up; Escape clears; tap a suggestion | Keyboard and on-screen keyboard behaviour |
| Results view | `main.rs` (`draw_canvas`, `touch`, `Scroller`) | Every dictionary's definition, wrapped, scrolled by drag (kit `Scroller`) | Scroll feel and the empty-state texts at phone width |
| Look Up Clipboard (cmd+l), Copy Definition, Clear Search | `main.rs` (`menu`, `menu_action`) | Done | Menu bar placement |
| Paste into the search field | `main.rs` (`event`, `Command::Paste`) | Done | - |
| Look Up intent | `main.rs` (`intents`, `perform_intent`), `manifest.toml` `[[intent]] look-up` | Returns the first definition; fails with a message when none | Shortcuts and search listing |
| Look Up in Dictionary text service (reference `NSServices` entry) | not wired | **Open**: needs a shell services entry that sends the selection to this app's `look-up` intent. The shell has no text-service menu yet; Books has a Look Up menu item with its own notice | Shell work, owned by the services slice |
| System word list spelling answer | `dict.rs` (`Library::contains_word`) | Done; reads `/usr/share/dict/words` when present (absent on the host) | - |

Dictionary folders: `<data dir>/dictionaries` (the app's own) and `/usr/share/stardict` (system). A broken
`.ifo` is listed in the footer as "could not be opened" and does not stop the others. Not supported yet:
compressed `.dict.dz` and `.idx.gz` files (reported as missing data), and dictionary management UI.

## Stickies

| Feature | BifrOSt path | Done | Still needs Midget checking |
|---|---|---|---|
| Six colours | `model.rs` (`Colour`), `main.rs` (colour menu) | Existing | Colour menu tick on the focused note |
| Translucent when inactive | `board.rs` (`INACTIVE_ALPHA`, `widgets::material`) | Existing | Look over a busy wallpaper |
| Collapse to the title bar | `model.rs` (`frame_h`, `part_at`), `main.rs` (disclosure touch) | Existing | - |
| Bold, Italic, three sizes | `model.rs` (`Style`), `board.rs` (`body_type`) | Existing; whole-note only (per-range formatting not kept) | - |
| Persist on every change | `store.rs` (atomic save, `.bad` on a broken file) | Existing | - |
| Import and export as text | `main.rs` (`import_panel`, `export_panel`, `document`), `model.rs` (`import_text`, `export_text`) | Existing | Save and open panels |
| Duplicate Sticky Note (cmd+d) | `model.rs` (`Board::duplicate`, `DUPLICATE_OFFSET_PT`), `main.rs` (`MENU_DUPLICATE`) | Done, with a unit test | - |
| Escape leaves the note | `main.rs` (`key`, `Code(27)`) | Done | - |
| Floating notes on top of the desktop, one window each | not done | **Open**: the SDK has no per-window level for apps, and each note is drawn on one board window. A window per note needs a multi-window app API | Needs the SDK window-level work |
| Caret-aware editing (type or delete in the middle of text) | not done | **Open**: typing always appends at the end; the board keeps no caret position | Model and draw change, next round |

## Open items for the lead

- Add `apps/dictionary-desktop` to the workspace (done in `springboard/Cargo.toml`). `Cargo.lock` needs a new
  entry for `bifrost-dictionary` and the `bifrost-stickies` version change; cargo writes both on the next build.
- Matrix closures (lead): "Dictionary" to `apps/dictionary-desktop` (partial: the Look Up text service is open);
  "Stickies" to `apps/stickies-desktop` (partial: per-note floating windows and caret editing are open).
- The icon for the dictionary app is a generated placeholder (`apps/dictionary-desktop/icon.png`); a real icon
  should replace it. The file is force-added because `.gitignore` excludes `*.png`.
