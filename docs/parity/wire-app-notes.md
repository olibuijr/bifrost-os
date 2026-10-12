# Wiring the text parity models into Notes (wire/app-notes)

Notes 1.10.0 (`springboard/apps/notes-desktop`) is the first app to use the text models from the parity waves
(`bifrost_app::controls::textfind`, `detect`, `language`; `bifrost_app::spellcheck`). The adapter is
`src/notes/parity.rs`; the editor calls it from `src/notes/mod.rs`. Ranges are char offsets, as the text area expects.

## What is wired

| Matrix row (`docs/parity-matrix/data/<slug>.json`) | Feature | Where it is in Notes | How to verify on the test PC |
|---|---|---|---|
| `appkit-text`: TextKit: custom layout, text attachments, text lists, find bar | Find with whole-word matching (`textfind::find_all`, `Options::whole_word`) | Open a note, then the editor More menu > Match Whole Word in Find | Note text "rust rusty rust": the find counter reads 3 of 3; with Match Whole Word on it reads 2 |
| `appkit-text`: Text Display: spell checking (`spellcheck`), and `datadetection`: Match base type and string scanning, Web links, Phone numbers, Calendar events and dates | Spelling merged with links, dates and phone numbers (`spellcheck::SpellChecker` over the keyboard app word lists, `detect::find`) | Editor More menu > Check Spelling and Links; the find arrows step through the flagged items | Type "wrold see https://example.com on 2026-10-09": the link and the date are flagged, and "wrold" is flagged in an English note |
| `natural-language`: Language identification (`NLLanguageRecognizer`, `NLLanguage`) | Note language (`language::dominant_language`) | The date line under the title shows the language, for example "Icelandic" | Type an Icelandic sentence (for example "Þetta er texti og það er gott að hafa hann með sér í dag."): the date line reads "... - Icelandic" |

Spelling uses the English or Icelandic word list (`springboard/apps/keyboard/src/words/{en,is}.txt`, included at
compile time). A note in another language gets only the links, dates and phone numbers.

## How a user reaches it

- Find: the find bar of a note (Find, Cmd+F, or the search field). Match Whole Word is in the editor's More menu.
- Check Spelling and Links: the editor's More menu. The find arrows step through the results. The next edit ends
  the check, the same as a search.
- Language: automatic, shown on the date line.

## Verify on the test PC (Midget only)

1. Publish and update Midget (`bifrost pc publish`, then `bifrost pc midget ota` until `phase=accepted`), then open
   Notes on it.
2. Create a note with the text in the table, open it, and take `bifrost pc midget screen /tmp/notes.png`.
3. Open the More menu with `bifrost pc midget tap-window Notes X Y` (window-local px from the screenshot) and
   check the rows above.
4. Host check: `cargo test` in the notes crate (the adapter tests in `parity.rs` and the new test
   `whole_word_find_and_check_spelling_step_through_their_matches`). Not run in this round: the lead builds once.

## Not wired (gaps)

- Replace (`textfind::replace_all`, `TextFinder::replace_selected`): the find strip has no replacement field. The
  adapter does not expose it yet; this needs a second field in `chrome.rs`.
- Lists (`textlist::Format`): Notes uses its own Markdown paragraph styles (bullet, numbered, checklist). The
  text area draws those markers, so the textlist formats (circle, square, roman, letters) cannot be used.
- Paragraph styles and attachments (`run_delegate`, the `CTRunDelegate` model): the attachment images are inline
  Markdown image lines (`attach.rs`), not run delegates. Paragraph styles stay the Markdown block styles.
- Spelling is a pass, not an underline: the text area only draws search highlights, so misspellings are
  stepped through, not underlined as the user types.
- The language recognizer is weak on short text and cannot tell Danish, Norwegian and Swedish apart reliably.
- Spelling correction (`SpellChecker::correction`) is not offered to the user.

## REA evidence (official documentation, rendered page)

- `ev_6e5bec17b36a0a15949bf8a8841a081c537a65c7936b3bf0890c3fbde0eba4a2`: NSTextFinder (appkit/nstextfinder)
- `ev_d3dbfc59c648a4dce618a1bd6b74dad430f7d182836e159bf6f15fa1507fec08`: NSSpellChecker (appkit/nsspellchecker)
- `ev_d395edf3d684a353a2310e93fb51c1b51dd203797bd2f247804133256f5b4585`: NLLanguageRecognizer (naturallanguage/nllanguagerecognizer)
- `ev_469daef39269e6b9fd490076598dab36610b60536dda9335fa717faff4af7d3d`: NSDataDetector (foundation/nsdatadetector)
