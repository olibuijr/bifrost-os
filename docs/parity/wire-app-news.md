# Wire app: News (language, tokens, link cards, Reading List)

News (`springboard/apps/news-desktop`, version 1.3.0) now uses three parity models that were built but not wired:
language identification and word tokenizing for article text, the rich link presentation model, and the Reading
List model. Nothing in the shell or the SDK changed; the wiring is the app's own code in `src/wire.rs`, which
adapts the models to the feed items the app already holds.

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | Model | Where it is used in News |
|---|---|---|---|
| natural-language | Language identification (NLLanguageRecognizer, NLLanguage) | `ui/src/language.rs` (`dominant_language`, `Language`) | The article's facts line names the language (for example "Icelandic") when the text has recognisable letters. |
| natural-language | Tokenizing text (NLTokenizer) | `sdk/src/spellcheck.rs` (`words`, the word units) | The facts line gives the word count and the reading time (200 words a minute, at least one minute). Scope: word units only. |
| link-presentation | Rich link view (LPLinkView) | `ui/src/linkmeta.rs` (`LinkView::loaded`, `LinkMetadata`, `Card`) | Under each article, the original page is a card with the article title and the feed name, in place of the bare address. The feed supplies the metadata; no request is made. |
| safari-services | Safari Reading List (SSReadingList) | `ui/src/reading_list.rs` (`ReadingList`) | A "Reading List" row in the source list (after Today). A toolbar button on an open article adds it, or removes it when it is already saved. Saved pages are kept in `reading-list.txt` in the app data folder (the 500 newest). |

Sidebar and tabs: tab 0 is Today, tab 1 is the Reading List, and tab n + 2 is feed n. The source list rows are
published as accessibility controls with the same ids (`100 + tab`). The toolbar button has id 3 (`BAR_READ`).

Store: `src/wire.rs` `load_reading_list` and `save_reading_list`. A missing or damaged file gives an empty list; a
failed write loses only that change.

## How a user reaches it

- Open News. The source list shows Today, Reading List, then Sources.
- Click an article. The facts line under the date shows, for example, "Icelandic  312 words  2 min read". The link
  card sits under the body.
- The toolbar shows "Add to Reading List" (dimmed for a page without an http or https address). Pressing it changes
  the label to "Remove from Reading List".
- Click the Reading List row to see the saved pages, newest first. An empty list says "Nothing saved yet".

## How to verify on the test PC (Midget)

1. `cd springboard && cargo build --offline --profile fast --bin bifrost-news` (lead build), then deploy with the
   app flow in `docs/bifrost-pc-workflow.md`, or `bifrost pc midget ota` for a system build that carries the app.
2. `bifrost pc midget open-app is.olibuijr.news.desktop`, then `bifrost pc midget screen /tmp/news.png`.
3. Click an article, then `bifrost pc midget tap-window News X Y` on the toolbar button (window-local pixels).
   Check the label flips and the Reading List row shows the article.
4. Restart News (`bifrost pc midget key` is not needed; quit and reopen) and confirm the Reading List persists.
5. Check the facts line on an Icelandic article (ruv.is) and an English one; check the link card title.
6. Light and Dark: `bifrost pc midget appearance light|dark`, then the same screenshots.
7. Host preview: `bifrost pc smoke news-desktop --light` and without `--light`.

## Tests

Unit tests in `src/wire.rs` (`cargo test -p bifrost-news wire`): language of an English and an Icelandic sentence,
word counting by the SDK tokenizer, reading time rounding, the link card (and refusal of `javascript:` and empty
links), toggling the Reading List with a round trip through the store text, refusal of an `ftp:` address, and a
missing store. These tests were written by reading only; the build has not run them.

The compliance gate (`tests/compliance.rs`) compares a snapshot; the new Reading List row changes the sidebar in
the golden, so the snapshot needs `BIFROST_BLESS=1` after a visual check.

## REA evidence

- `ev_519dcbd7ce845db9cb6326190c4e4a1aa69cfaa0cf441fb5d19527ff660498f8`: rendered NLLanguageRecognizer page
  (`https://developer.apple.com/documentation/naturallanguage/nllanguagerecognizer`): dominant language, hypotheses,
  hints and constraints, and the note that short text is less reliable.
- `ev_f68fadb299186b778e60162a2b98be6facb9d454e81b727739ffed882010a3b2`: rendered NLTokenizer page
  (`https://developer.apple.com/documentation/naturallanguage/nltokenizer`).
- `ev_4ecb82343635797988fd1e84b1d52dc3e1a752a163553ef25c33ca94cc11a967`: rendered Link Presentation page (from theme-24).
- `ev_a78c80dd67e48d793cee4c4f5446d4aa9d0db04c06f40d51af20509bab326bb7`: rendered Safari Services page with the
  Safari Reading List section (from the Wave 1 note).

DocC JSON was read with `python3 -I tools/parity/docc.py naturallanguage/nltokenizer` (the path form without a
`documentation/` prefix; the prefixed form returns 404).

## Remaining gaps

- Language is only a label. Hypotheses, hints and constraints are not used, and the list does not filter by language.
  Tokenizing has no sentence or paragraph units and no CJK segmentation (the word units only).
- The link card does not open the page, and it shows no thumbnail (the hero picture is the article's image). The
  `LinkMetadata` fetch (`MetadataProvider`, `FetchPolicy`) is not used: the app still scrapes og:image itself in
  `src/pictures.rs`.
- The Reading List has no sync, no search, no removal by swipe, and no share target. Its rows reuse the headline
  row layout, so the picture is loaded from the saved page's address.
- Version 1.3.0 is in `manifest.toml` and `Cargo.toml`. `springboard/Cargo.lock` still lists `bifrost-news` 1.2.2
  (outside this app's folder); the lead's build updates it, or it needs a `cargo update -p bifrost-news` before a `--locked` build.
- The matrix rows in `docs/parity-matrix/data/` were not changed (per the brief).
