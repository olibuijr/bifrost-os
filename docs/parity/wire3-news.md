# Wire 3: News (translation of an article, Edit menu, content colour)

News (`springboard/apps/news-desktop`, version 1.4.0) gets the Translation models wired into the reader. The
previous News wiring (`docs/parity/wire-app-news.md`, 1.3.0) already wires language identification, word tokens
with reading time and the rich link card, so this round leaves those as they are. The new work is:

- Translate Article in the Edit menu, which runs the Translation session for an open article.
- A notice under the article's facts line that says why a translation could not run.
- The reader's background moves from `DETAIL_BG` (pure black in Dark) to `CONTENT_BG` (dark grey in Dark, white in Light).

No shell, SDK or kit file changed. Everything is in the app's own `src/` folder.

## What is now wired

| Matrix slug | Feature (as in data/<slug>.json) | Model | Where it is used in News |
|---|---|---|---|
| translation | Translation session and tasks | `sdk/src/translation.rs` (`TranslationSession::new`, `translate`), `ui/src/translation.rs` | `wire::translate_article` opens a session for the article's pair and translates its title and preview. |
| translation | Language availability | `ui/src/translation.rs` (`LanguageAvailability::new`, `check_session`, `status`) | `wire::installed()` gives the pairs News knows (English and Icelandic) with nothing installed. The session check uses it. |
| translation | Translation errors | `ui/src/translation.rs` (`TranslationError::error_description`, `failure_reason`) | The notice under the facts line shows the error's description and reason, for example "The translation languages are not installed. The languages are not downloaded on this device." |
| natural-language | Language identification (NLLanguageRecognizer, NLLanguage) | `ui/src/language.rs` (`dominant_language`, `Language::code`) | Used by `wire::translation_plan` to find the source language of the article (the target is the other of English and Icelandic). Facts line wiring is from 1.3.0. |

Not wired this round: `translationPresentation` (the system popover, a gap), `TranslationSession.translate(_:)`
for formatted text (`translate_attributed`), the batch call, and `cancel()`. None of them is needed for the
article command.

Link previews (`link-presentation`) and the other natural-language rows (tokenizing, script, tagging, embeddings)
are unchanged here. The link card and the facts line were wired in 1.3.0.

## How a user reaches it

1. Open News and click an article in Today or a feed.
2. Open the Edit menu (in the menu bar, while News is in front). Choose Translate Article. The item is greyed out
   until an article is open.
3. The article's title and preview are translated from Icelandic to English, or from English to Icelandic.
   Articles in another language are refused: "The source language is not supported." Articles with no
   recognisable letters are refused: "The source language could not be identified."
4. Today, BifrOSt ships no translation language packs, so the Icelandic to English and English to Icelandic
   pairs report "The translation languages are not installed." under the facts line, with the reason "The
   languages are not downloaded on this device." The article itself does not change.
5. Opening another article, or going Back, clears the notice.

## How to verify on the test PC (Midget)

1. Lead build: `cd springboard && cargo build --offline --profile fast --bin bifrost-news`, then deploy as in
   `docs/bifrost-pc-workflow.md`, or `bifrost pc midget ota` for a system build that carries the app.
2. `bifrost pc midget open-app is.olibuijr.news.desktop`, then `bifrost pc midget screen /tmp/news.png`.
3. Open an Icelandic article (ruv.is). Click the menu bar's Edit menu and check that "Translate Article" is enabled.
   Choose it. The notice "The translation languages are not installed.  The languages are not downloaded on
   this device." appears under the facts line, and the article text is unchanged.
4. Open the Edit menu with no article open: "Translate Article" is greyed out.
5. Go Back to the list and check that the notice is gone.
6. Check the background of the article page is the content colour (`bifrost pc midget appearance light`, then
   `dark`): in Dark it is dark grey, not black, and the source list and toolbar keep their own colours.
7. Host preview: `bifrost pc smoke news-desktop --light` and without `--light`.

## Tests

Unit tests in `src/wire.rs` (`cargo test -p bifrost-news wire`):

- `translation_plan_picks_the_other_of_english_and_icelandic`: an Icelandic sentence plans `is` to `en`, an English
  sentence plans `en` to `is`, blank text gives `NothingToTranslate`, and text with no letters gives
  `UnableToIdentifyLanguage`.
- `no_language_pack_is_installed_so_a_translation_reports_not_installed`: with `installed()`, the Icelandic article
  gives `NotInstalled`, and the pair's status is `Supported` (not installed).
- `an_installed_pair_translates_the_title_and_preview_through_the_engine`: with the pair installed and a
  `TranslationMemory` holding the title and the preview, the result is the two phrases.
- `a_phrase_missing_from_the_engine_is_an_internal_error`: an installed pair with an empty phrase table gives
  `InternalError`.

Expected values were worked out by hand from `ui/src/translation.rs` (the session check runs supported, then
target, then pairing, then installed). The language recogniser's result for the Icelandic test sentence is the same
one the 1.3.0 tests rely on. These tests were written by reading only; the build has not run them.

The compliance gate (`tests/compliance.rs`) compares a snapshot. The reader background change and the new Edit item
change the golden, so the snapshot needs `BIFROST_BLESS=1` after a visual check on the host preview.

## REA evidence

- `ev_4f175a849c8f943021fce340a06be056e2162df5ce54dd8a2d880d842babf71c`: rendered Translation framework page
  (`https://developer.apple.com/documentation/translation`), with the translation popover, translation task, the
  Translation Session, Language Availability and Translation Error sections. Read with `tools/parity/rea-page.sh`.
- `python3 -I tools/parity/docc.py translation`: the framework's topic list (presentation, task, session,
  availability, errors).
- Matrix evidence already on file, for the rows touched here: `ev_318b6155181a029fffc215b27f3997bb92800b4751200f5d8b7a005c5a848088`
  (Translation), `ev_9e68febdd223107558efdf4b30d4323aefbf32ccac1dce92a912c41429217f7b` (Natural Language).

## Remaining gaps

- No language packs and no translation engine ship with BifrOSt. The Translation session runs only with a
  `Translator` the app supplies; News supplies an empty `TranslationMemory`, so the command reports
  `NotInstalled` and no text is translated yet.
- No system translation popover (`translationPresentation`) and no inline translate action for a selection.
- Only the article title and preview are translated, as plain text. Formatted-text translation
  (`translate_attributed`) is not used, and the reader does not keep the original text to switch back.
- No batch translation of the headlines in a list, no cancel on a long request, and no download-status
  query in the UI.
- Language recognition is a label and a planning step only, as in 1.3.0.
- `springboard/Cargo.lock` still lists `bifrost-news` 1.3.0 (outside this folder). The lead's build updates it.
- The matrix rows in `docs/parity-matrix/data/` were not changed (per the brief).

## Matrix rows touched

| slug | feature (as in data/<slug>.json) | BifrOSt path |
|---|---|---|
| translation | Translation session and tasks | `springboard/apps/news-desktop/src/wire.rs` (`translate_article`), `springboard/sdk/src/translation.rs`, `springboard/ui/src/translation.rs` |
| translation | Language availability | `springboard/apps/news-desktop/src/wire.rs` (`installed`), `springboard/ui/src/translation.rs` |
| translation | Translation errors | `springboard/apps/news-desktop/src/news.rs` (`translate`, notice), `springboard/ui/src/translation.rs` |
| natural-language | Language identification (NLLanguageRecognizer, NLLanguage) | `springboard/apps/news-desktop/src/wire.rs` (`translation_plan`, `facts`), `springboard/ui/src/language.rs` |
| link-presentation | Rich link view (LPLinkView) | `springboard/apps/news-desktop/src/wire.rs` (`link_card`), unchanged from 1.3.0 |
