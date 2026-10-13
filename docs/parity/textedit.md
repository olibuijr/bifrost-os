# Text Editor (textedit slice, round 75)

App: `springboard/apps/textedit-desktop` (bifrost-textedit 1.1.0). Branch work only; the lead builds and merges.

## Reference evidence used

- `macos-ref/extract/ref27/System/Applications/TextEdit.app/Contents/Info.plist`: document types. Rich text (`public.rtf`),
  HTML (`public.html`), plain text (`public.plain-text`) are the Editor types. The app opens and saves those three.
- `.../Resources/Base.lproj/Edit.nib` (read with `strings`, not REA): menu titles for Edit (Find submenu, Spelling and
  Grammar: Check Document Now, Check Spelling While Typing, Correct Spelling Automatically; Substitutions; Transformations),
  Format (Font, Text, Make Rich Text, Show Ruler, Page Setup, Print).
- `.../Base.lproj/PrintPanelAccessory.nib` and `Preferences.nib` exist; not decoded in this round.
- No REA evidence ids were recorded in this round.

## Per-row status

- Text Display: spell checking (NSSpellChecker): BifrOSt path `springboard/apps/textedit-desktop/src/spell.rs` over
  `bifrost_app::spellcheck` (SDK re-export of `bifrost_core::spellcheck`), learned words in the app preferences.
  Done: misspelled words underlined while typing (`view::draw_text` squiggle, cached in `Cache.misspelled`), Check
  Document Now (next misspelled word), Correct Spelling (top guess, keeps the word's style), Learn Spelling, Ignore Spelling,
  Check Spelling While Typing toggle. Needs Midget: the squiggle colour and placement, learned words after a relaunch.
- Text Display: multi-line text view (NSTextView): the window still draws its own cell model (`text.rs`, `layout.rs`).
  Not moved to `bifrost_ui::typeset` or `RichTextView` in this round. Open.
- Core Text: paragraph styles and tab stops (CTParagraphStyle, CTTextTab): app-level only. `layout.rs` wraps with line
  spacing, alignment, list indents and tab stops; the ruler above the page sets the tab distance (`view::draw_ruler`,
  `view::ruler_tab`). The SDK attributed model still has no paragraph styles. Needs Midget: ruler click and tab
  stops in the window and in the print output.
- Printing (print operation and page setup): `springboard/apps/textedit-desktop/src/print.rs` writes a PDF (standard
  faces, WinAnsi), paginated to `PrintInfo::imageable()`; `main.rs` sends it through `bifrost_app::print::PrintOperation`
  to the default printer. File > Print, File > Paper Size (A4, US Letter, US Legal, A5, A3), File > Orientation.
  Needs Midget: a real `lp` queue, and the page breaks against the window's line breaks.
- Document formats: `src/document.rs` reads by bytes (RTF header, HTML prologue, else plain text) and writes the type the
  save uses. `src/rtf.rs` (RTF read and write, Windows-1252 escapes, UTF-8 bytes), `src/html.rs` (HTML read and write,
  inline styles, lists, entities; its own codec because the SDK's attributed HTML loses alignment and lists).
  Save As picks the type by the extension typed (`type_for_extension`). Open and Save of .txt, .rtf and .html need Midget.
- Rich and plain mode: Format > Make Plain Text / Make Rich Text and the format bar's mode button. Plain text disables the
  formatting commands and hides the ruler. The switch is one undo entry.

## Migrations and deletions

- `view::geometry` gained the ruler flag; `main.rs` passes whether the document is rich.
- `layout::layout` (no tab stops) is kept for tests; the window and print use `layout::layout_tabs` with the document's tab.
- Five copies of "two styles share a run" (text runs in view, RTF, HTML and print) are one `Style::same_run` in `text.rs`.
- Two copies of `nearest_spacing` (RTF and HTML) are one, in `rtf.rs`, imported by `html.rs`.

## Shared-file edits the lead must merge carefully

- None in the SDK, the UI kit or the shell. The save panel offers only the document's current type as a filter (`sdk/src/document.rs`
  `start_panel`). Save As into .txt or .html still works by typing the extension, but the picker will not list them.
  Changing that filter is an SDK change that affects every document app, so it is left for the lead.

## Still open

- Move the layout and the caret onto `bifrost_ui::typeset` and `RichTextView` (the brief's "new text engine"). Not done;
  the cell model and `layout.rs` are the working path.
- Snapshot PNGs in `springboard/apps/textedit-desktop/tests/snapshots/` no longer match (format bar and ruler). The lead
  re-blesses them after the build.
- Matrix rows: none closed by this slice. Spell checking is wired into an app for the first time, which is evidence for
  the spell-checking row; the lead decides once Midget confirms it.
