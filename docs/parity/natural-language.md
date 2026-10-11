# Natural Language parity (wave 2)

Official reference: https://developer.apple.com/documentation/naturallanguage (macOS 10.14 and later).

## What was built

Pure Rust, std only, in `springboard/ui/src` (bifrost-ui), no new crates and no unsafe.

* `springboard/ui/src/language.rs`: language and script identification.
  * `LanguageRecognizer` mirrors `NLLanguageRecognizer`: `process` (processString), `reset`, `dominant`
    (dominantLanguage), `hypotheses` (languageHypotheses(withMaximum:)), `set_hint` (languageHints) and
    `set_constraints` (languageConstraints). `dominant_language(text)` is the one-shot form (dominantLanguage(for:)).
  * `Language` mirrors the `NLLanguage` codes for 20 languages (`code` gives the BCP 47 tag, `from_code` the reverse).
  * `Script` mirrors `NLScript` for 12 writing systems, classified by Unicode block (`of`, `dominant`).
  * Method: Latin-script text is scored by common function words per language and by letters that point at one
    language (for example the Icelandic eth and thorn). Single-script text (Greek, Hebrew, Arabic, Thai, Hangul,
    Cyrillic as Russian, kana as Japanese, Han as Chinese) is scored from its letters. Hypotheses are the scores
    normalised over the candidates.
* `springboard/ui/src/segment.rs`: segmentation for `NLTokenizer`. `Unit` mirrors `NLTokenUnit` (word, sentence,
  paragraph, document), and `tokens(text, unit)` returns UTF-8 byte ranges.
  Sentence and paragraph units are new. The word unit follows the same rule as `springboard/sdk/src/spellcheck.rs` `words`.

Both files are registered in `springboard/ui/src/lib.rs` (one `pub mod` line each). Tests: 9 in `language.rs`
and 7 in `segment.rs`, all behaviour tests. No builds were run for this wave.

## Official API names mirrored

* NLLanguageRecognizer: init(), processString(_:), reset(), dominantLanguage, dominantLanguage(for:),
  languageHypotheses(withMaximum:), languageHints, languageConstraints.
  Source: `python3 -I tools/parity/docc.py naturallanguage/nllanguagerecognizer`.
* NLLanguage: standard language identifiers (subset: 20 of the 40-plus Apple languages).
  Source: `docc.py naturallanguage/nllanguage`.
* NLScript: standard script identifiers (subset: 12). Source: `docc.py naturallanguage/nlscript`.
* NLTokenizer: init(unit:), unit, enumerateTokens(in:using:) and tokens(for:) (as `tokens(text, unit)`, not a
  range API), NLTokenUnit word, sentence, paragraph, document.
  Source: `docc.py naturallanguage/nltokenizer`.

## REA evidence

* Landing page, rendered headings (Tokenization, Language identification, Linguistic tags, Text embedding, Contextual
  embedding, custom models): `ev_c2d3050c0a605d8af8acf43ed91deb170d604c48f02f83ddd217e01aa038b30c`
  (`tools/parity/rea-page.sh https://developer.apple.com/documentation/naturallanguage`).

## Matrix rows closed (data/natural-language.json)

| Framework | Feature (as written in data/natural-language.json) | New status | BifrOSt path |
|---|---|---|---|
| Natural Language | Language identification (NLLanguageRecognizer, NLLanguage) | partial | springboard/ui/src/language.rs (LanguageRecognizer, Language) |
| Natural Language | Script detection (NLScript) | partial | springboard/ui/src/language.rs (Script) |
| Natural Language | Tokenizing text (NLTokenizer) | partial | springboard/ui/src/segment.rs (Unit, tokens); the word unit stays in springboard/sdk/src/spellcheck.rs |

The Tokenizing row stays partial: sentence and paragraph units now exist, but there is no locale-aware segmentation
and no UAX #29 table.

## Remaining gaps

* Parts of speech, lexical class and lemma (NLTagger lexicalClass): no tag data.
* Named entity recognition (NLTagger nameType): data detectors cover only link, date, address and phone.
* Text embedding (NLEmbedding) and contextual embedding (NLContextualEmbedding): no vector model.
* Custom text models (NLModel): no runtime.
* Language recognition limits: no n-gram model. Danish, Norwegian and Swedish share most function words and are not
  reliably separated. Short texts with one or two function words are weak evidence. Cyrillic text is returned as
  Russian without checking for other Cyrillic languages.
