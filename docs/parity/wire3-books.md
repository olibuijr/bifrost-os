# Wire 3: Books (Read Aloud, Look Up, Translate)

Round: wire the speech, natural-language and translation models into the native Books app
(`springboard/apps/books-desktop/`), version 1.4.0. Branch `wire3/books`, from `main` (release 0.5.38). Not built
here: the lead builds once (`cargo test -p bifrost-books` in `springboard/`, then the compliance and snapshot tests).

## What is now wired

| Feature (matrix slug, feature) | BifrOSt path | Status after this round |
|---|---|---|
| `avfaudio`, "Speech synthesis (AVSpeechSynthesizer)" | `springboard/apps/books-desktop/src/books/narrate.rs` (`Narration`) over `springboard/ui/src/speech.rs` (`Synthesizer`, `Utterance`, `RATE_DEFAULT`) | Partial: the queue and the play, pause, resume and stop states are the model's. No audio: nothing is spoken (see gaps). |
| `avfoundation-capture-audio`, "Speech synthesis" | same as above | Partial, same as above. |
| `natural-language`, "Language identification (NLLanguageRecognizer, NLLanguage)" | `narrate.rs` (`look_up_notice`, `translate_notice`), `books/mod.rs` (`look_up`, `translate`) through `bifrost_app::controls::language::dominant_language` (`springboard/ui/src/language.rs`) | Partial: the language of the sentence is named in Look Up and Translate. |
| `translation`, "Translation session and tasks" | `narrate.rs` (`translate_notice`) through `bifrost_app::controls::translation::LanguageAvailability::check_session` | Partial: the session check runs before a translation would start. No translation is performed: no language packs exist. |
| `translation`, "Language availability" | same as above (`LanguageAvailability::new`, `check_session`) | Partial: no supported languages are listed, so every pair reports unsupported. |
| `translation`, "Translation errors" | `narrate.rs` (`translate_notice`, `TranslationError::error_description`) | Partial: the framework's error text is shown to the person. |

Read Aloud also reuses the reader's existing paging, the sentence split (`narrate::sentence_spans`) and the chapter
range (`narrate::chapter_end`, from the table of contents). Look Up and Translate act on the text the last find went
to (the same match Copy Quote uses), because the reader has no free text selection yet.

Note on the Translate row: the matrix's "Formatted-text translation" row is not changed. No `translate` call is made
and no text is translated.

## How a user reaches it

Open a book in Books. The **Find** menu gains, after **Copy Quote**:

- **Look Up** and **Translate**: enabled after a find has a match (the match is highlighted). The result is a one-line
  card at the top of the page. A tap on the page clears it.
- **Read Aloud**: enabled with a book open. It reads from the page on to the end of the chapter. The label becomes
  **Pause Reading**, then **Resume Reading**. While a chapter is read, the sentence being read is shaded in the tint,
  the book turns pages to follow it, and a card on the page says that no voice is installed.
- **Stop Reading**: enabled while a chapter is read. It ends the reading.

The Find menu is the only new menu surface. There is no second Edit menu. The library's main area now uses
`CONTENT_BG` (white in Light, dark grey in Dark) instead of `DETAIL_BG`.

## Verify on the test PC (Midget)

Use Midget only (no VM). Build with the fast profile, publish, then `bifrost pc midget ota` until `phase=accepted`.

1. `bifrost pc midget state`: Books is running at 1.4.0.
2. Open a book with several chapters. Open the Find menu: Look Up, Translate, Read Aloud, Stop Reading and Copy Quote
   are listed; Stop Reading is disabled.
3. Choose **Read Aloud**. The first sentence is shaded and the card shows the no-voice note. After about a few seconds the
   shading moves to the next sentence. Choose **Pause Reading**: the shading stops. Choose **Resume Reading**: it goes
   on. Turn to a page the reading is not on: the book turns back to the sentence being read when it moves on.
4. Choose **Stop Reading**: the shading and the card go, and Stop Reading is disabled again.
5. Search for a word, press ⌘G to a match. Choose **Look Up**: the card names the word and the language (for an
   Icelandic book, "Icelandic"), and says there is no dictionary. Tap the page: the card goes.
6. Choose **Translate** on the same match: the card says "Translate from <language>: The source language is not
   supported." Tap to dismiss.
7. Check the library: the main area is white in Light and dark grey in Dark (`bifrost pc midget appearance light|dark`).
8. Screenshot with `bifrost pc midget screen PATH` and check the Find menu and the card against `bifrost-hig`
   (one tint, no second accent).

Host tests (lead's build): `cargo test -p bifrost-books narrate_tests` from `springboard/`. They check sentence splitting
(stops followed by a space, decimals, ellipsis, from-offset, blank text), chapter ends, reading time per sentence at the
default and extreme rates (150, 240 and 60 words a minute, 600 ms floor), the timing of advances, pause and resume with
the time left, the late catch-up tick, and the Look Up and Translate notice text. Snapshot tests
(`tests/snapshots/books-desktop-*.png`) change because the library background moved to `CONTENT_BG`; the lead blesses them.

## REA evidence

- `ev_e5e3c20e8552fec6ac06402623e73558f4da0a949f8f5a753dffea5cc9ad710b`: AVSpeechSynthesizer (official DocC page,
  rendered; the queue, speak, pause, continue, stop, isSpeaking and isPaused).
- `ev_e4b253cdc575499303113cb54af660e25f9f8cc2d50a592dfa8ac43d4e5ec6ca`: TranslationSession (official DocC page,
  rendered; `init(installedSource:target:)` throws when the languages are not installed; `isReady`, `translate`).
- `ev_cb5efe23006c191cc5e85b8ff55b5b2b2e77ef699260d0163f6776f94d9f7cb0`: NLLanguageRecognizer (official DocC page,
  rendered; `dominantLanguage(for:)`, `processString(_:)`, `languageHypotheses(withMaximum:)`).

Official references read with `python3 -I tools/parity/docc.py avfaudio/avspeechsynthesizer`,
`translation/translationsession` and `naturallanguage/nllanguagerecognizer`.

## Remaining gaps

- **No audio.** There is no text-to-speech engine or SDK call that speaks. The highlight follows an estimate of the
  reading time, and the card says no voice is installed. Wiring the sound needs a speech service in the system layer
  (a voice engine that takes an utterance and reports when it ends, so `finish_current` is driven by the engine, not
  the timer). The model is ready for that.
- **No translation.** No language packs ship, so `LanguageAvailability` lists no languages and every pair is
  unsupported. The formatted-text translation (`translate_attributed`) is not called.
- **No dictionary.** Look Up gives the language and says there is no definition. A dictionary source is needed before
  Look Up can define a word.
- **Read Aloud starts at the current page**, not at the chapter's first sentence, and reads to the next table of
  contents entry (or the end of the book when the book has no contents list).
- **No free text selection in the reader.** Look Up and Translate act on the match the last find went to, not on any
  selected text.
- **Speaking rate is fixed** at the model's default (`RATE_DEFAULT`); there is no setting for it.
- **Matrix rows are not changed** in this branch (`docs/parity-matrix/` is out of scope). After Midget verification,
  the rows listed above can be promoted with this note.

## Files

- `springboard/apps/books-desktop/src/books/narrate.rs` (new: `Narration`, `sentence_spans`, `chapter_end`,
  `duration_ms`, Look Up and Translate notices)
- `springboard/apps/books-desktop/src/books/narrate_tests.rs` (new tests)
- `springboard/apps/books-desktop/src/books/mod.rs` (`Reader.narration`, `Books.notice`, `read_aloud`, `stop_reading`,
  `look_up`, `translate`, `follow_narration` in `tick`, `next_wake`, the notice cleared on a tap)
- `springboard/apps/books-desktop/src/books/draw.rs` (`CONTENT_BG` library background, the read-aloud shading in
  `draw_page`, `draw_banner`)
- `springboard/apps/books-desktop/src/main.rs` (Find menu items and their actions)
- `springboard/apps/books-desktop/Cargo.toml`, `manifest.toml` (1.4.0), `CHANGELOG.md`
- `docs/parity/wire3-books.md` (this note)

`springboard/Cargo.lock` has the `bifrost-books` entry at 1.3.0. It is not edited here (outside the app folder); the
lead's build refreshes it.
