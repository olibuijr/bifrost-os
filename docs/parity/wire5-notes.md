# Wire 5: Notes find and replace, smart folders (apps/notes-desktop)

Scope: the Notes features that do not need rich text (Markdown storage stays, and the note file format is unchanged):
replace in a note, smart folders over predicate queries, and the spelling and link check that wire 2 and the app wiring
already connected. Owned folder: `springboard/apps/notes-desktop/`, plus this note. Version 1.13.0 (manifest.toml,
Cargo.toml, CHANGELOG.md). Branch `wire5/notes`.

## What is now wired

| Feature | Where | How a user reaches it |
|---|---|---|
| Replace in a note: a replacement card over the editor, Replace (the selected match, then on to the next) and Replace All (every match, one undo step) | `src/notes/mod.rs` (`start_replacing`, `replace_one`, `replace_all`, `draw_replacing`, `replace_buttons`, the key and touch branches), `src/notes/parity.rs` (`find_ranges` from `textfind`, `replace_ranges`) | Type the find text in the search field, open a note, More menu > Replace in Note. Type the replacement. Replace, Replace All, Enter (= Replace All), Escape or a tap outside the card closes it. Without find text the note says "Search for text first, then replace it". |
| Replace follows the find options | `parity::find_ranges` uses the note's whole-word setting | More menu > Match Whole Word in Find, then Replace |
| Smart folders over the note fields: Pinned, Edited This Week, Checklists, Photos | `src/notes/smart.rs` (`Smart`, `predicate`, `matches`, the `Fields` record over text, modified time and pin), `src/notes/mod.rs` (`View::Smart`, `visible`, `smart_cutoff`, `open_list_menu`), `src/notes/chrome.rs` (sidebar "Smart Folders" header and rows, `count_in`, `row_view`) | Sidebar > Smart Folders > a row, or the list More menu (the sort and folder menu) lists them with counts |
| Smart folder restore after a relaunch | `src/main.rs` (`view_key` and `view_from_key`: `smart:<key>`) | Automatic |
| Pinning (already wired before this wave, not changed) | `store::set_pinned`, `store::load_pins` | Row pin control, or More > Pin Note |
| Check Spelling and Links (already wired in wire 2 and the app wiring) | `src/notes/parity.rs` (`check_ranges`) | More menu > Check Spelling and Links |

Smart folder rules (the predicates in `smart.rs`, read through `bifrost_app::controls::fetch`):
- Pinned: `pinned == true`.
- Edited This Week: `modified >= now - 604800` (seven days; a change exactly at the cutoff counts).
- Checklists: the text contains `- [ ] ` or `- [x] ` (case-insensitive, so `- [X] ` also counts).
- Photos: the text contains `![` (an image line of the Markdown form `![image](name)`).
A note in Recently Deleted never shows in a smart folder. A smart folder is a view only: nothing is moved or written.

Replace and the storage format: Replace changes the note's Markdown text through the editor's `replace_range`, so the
file format is the same, and one Undo takes back a whole Replace All. The find model is `textfind::find_all` (through
`parity::find_ranges`). The splice is in char units (`parity::replace_ranges`), because the editor's ranges are chars;
`textfind::replace_all` works on byte ranges and was not used, so Replace All is one undo step from the editor instead.

## How to verify on Midget

1. `bifrost pc midget update --restart`, then `bifrost pc midget open-app is.olibuijr.notes.desktop`.
2. Replace: create two notes or use an existing one. Type `one cat, cat` and `the Cat sat` in a note. Type `cat` in the
   search field (the note's find strip shows "1 of 3"). Open the note, More menu > Replace in Note. The card shows at the top
   of the note with the field "Replacement". Type `dog`, tap Replace: the first "cat" becomes "dog", the counter reads 1 of 2.
   Tap Replace All (or press Enter): all matches change to "dog" (the case of "Cat" does not matter), the card closes.
   Press Undo once: the text returns to the state before Replace All.
3. Replace needs find text: clear the search field, open the note, More > Replace in Note. The note shows "Search for text
   first, then replace it" and no card opens.
4. Smart folders: in the sidebar, under "Smart Folders", the rows read Pinned, Edited This Week, Checklists and Photos with
   counts. Make a note with `- [ ] milk` and check that Checklists lists it. Pin a note: Pinned lists it. Edit a note:
   Edited This Week lists it. A picture note lists under Photos. Recently Deleted notes never show.
5. Relaunch the app with a smart folder open: it reopens on that folder (state restoration).
6. Screenshot with `bifrost pc midget screen`, and compare the replace card and the sidebar in Light and Dark
   (`bifrost pc midget appearance light|dark`).
7. Host tests (not run in this round; the lead builds once): `cargo test -p notes` covers `smart.rs` (`checklists_need_a_checkbox_line_in_either_case`,
   `recent_compares_the_last_change_with_the_cutoff`, `pinned_follows_the_pin_not_the_text`,
   `photos_need_an_image_line`, `keys_round_trip_and_unknown_keys_are_refused`), `parity.rs`
   (`replace_all_swaps_each_match_in_order`), and `mod.rs` (`smart_folders_pick_notes_by_their_fields`,
   `replace_one_and_replace_all_change_the_open_note`, `replace_needs_find_text_first`, and
   `sidebar_lists_folders_with_counts_and_switches_the_list`, whose sidebar row list now has seven rows: three folder rows
   and the four smart folders).
   Expected values were worked out by hand: "one cat, cat" with `cat` gives matches 4..7 and 9..12, so Replace All with
   `dog` gives "one dog, dog"; "one cat, cat\nthe Cat sat" has three matches (the third is "Cat" at 17..20), so Replace
   changes the first one, leaving two matches, and Replace All ends at "one dog, dog\nthe dog sat".
   The compliance snapshot (`tests/compliance.rs`, `tests/snapshots/notes-desktop-*.png`) changes because of the new
   sidebar rows; the lead blesses it.

## Reference evidence (official documentation, rendered pages)

- `ev_3eab27c220a08a8b2f2ec97d6a7fd1dcfdecf605de6107802593004d46d678c0`: NSTextFinder (appkit/nstextfinder). The text
  finder is the find bar controller; the "Replacing Text" section is the basis of Replace and Replace All.
- `ev_92bed9e184272e657dcc8708ca2d8761cf32f3dca8e4a6a2a4f9651d84534474`: NSFetchRequest (coredata/nsfetchrequest). A
  fetch request is a predicate, sort descriptors and a limit over records; the basis of the smart folder queries.
- `ev_4877812e3465fd950aa0d1990202cd08dd18637bf4efad33ffabbfdf131037f7`: NSPredicate (foundation/nspredicate). The
  predicate expressions the smart folders mirror: comparison, text containment, and the boolean combinators.
- `tools/parity/docc.py` returned the NSTextFinder and NSFetchRequest collection pages (`appkit/nstextfinder`,
  `coredata/nsfetchrequest`); the `documentation/...` form returned HTTP 404 from this session.

## Not wired, and why

- **A replace field in the find strip.** The find strip is a single 44 pt row above the keyboard (Previous, Next,
  Keyboard). Replace uses a card over the editor instead; a second row in the strip is a larger layout change.
- **Replace while Check Spelling and Links is shown.** The check sets the highlights without a find text, so Replace is
  refused there (the note asks for find text). This is deliberate: replacing a misspelling is the spelling correction,
  which is not offered yet.
- **Spelling correction (`SpellChecker::correction`) and guesses.** The check flags and steps through; no suggestion list.
- **Checklists, tables and attachments as models.** The notes are Markdown text. The checklist boxes are drawn by the
  shell's text area (`springboard/src/shell/textarea.rs`), outside this app, and the table model (`ui/src/table.rs`) is a
  data grid with no Markdown form, so neither is wired here. Attachments stay the Markdown image lines (`attach.rs`).
- **User-made smart folders.** The four folders are built in. An editor for predicates (a sheet with fields and
  operators) is not built; the predicate model supports it.
- **Smart folder counts** are taken at each list rebuild, not on a timer, so a count changes when the list changes.
- **Replace across all notes.** Replace works in the open note only, as the find bar does.
- **Pinning.** Already wired before this wave. No change.

## Matrix rows

| Slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| appkit-text | TextKit: custom layout, text attachments, text lists, find bar | springboard/apps/notes-desktop/src/notes/mod.rs (`start_replacing`, `replace_one`, `replace_all`, the replace card), springboard/apps/notes-desktop/src/notes/parity.rs (`find_ranges`, `replace_ranges`) |
| core-data | Fetch requests and predicates | springboard/apps/notes-desktop/src/notes/smart.rs (`Smart::predicate`, `Smart::matches`, `Fields`), springboard/apps/notes-desktop/src/notes/mod.rs (`visible`, `smart_cutoff`) |

## Parity status

The rows stay `partial` until the replace card and the smart folders are checked on Midget
(`bifrost pc parity`, skill `bifrost-parity-matrix`). No matrix data was edited in this round.
