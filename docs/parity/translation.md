# Translation framework: parity notes

Framework: Translation (macOS 15.0+). Official doc: https://developer.apple.com/documentation/translation.
Matrix row: `docs/parity-matrix/data/translation.json`. Status moves from `gap` to `partial`.

## What was built

Pure Rust, std only, no I/O, no network, no language packs.

- `springboard/ui/src/translation.rs` (`bifrost_ui::translation`):
  - `LanguageAvailability`: the supported language list (given by the caller, reduced to codes, de-duplicated),
    directional installed pairs, `status(from:to:)`, `install(from:to:)` and `check_session(from:to:)`.
    `check_session` reports the framework's error order: unsupported source, unsupported target, same-language
    pairing, then not installed.
  - `AvailabilityStatus`: `Installed`, `Supported`, `Unsupported`.
  - `TranslationError`: all eight framework cases, with `error_description()`, `failure_reason()`, `Display`
    and `std::error::Error`.
  - `language_code()`: reduces `en_US.UTF-8`, ` IS-is ` and `de@euro` to `en`, `is` and `de`.
- `springboard/sdk/src/translation.rs` (`bifrost_app::translation`):
  - `TranslationSession`: built from an availability check and an engine; `translate`, `translate_batch`
    (one result per input, in order), `cancel`, `is_cancelled`, `source_language`, `target_language`.
    Empty or whitespace-only text fails with `NothingToTranslate` before the engine runs. After `cancel`,
    every call fails with `AlreadyCancelled`.
  - `Translator`: the engine trait. It receives normalised codes and non-empty text.
  - `TranslationMemory`: a phrase table implementing `Translator`. Lookup is exact and directional.
    A miss returns `InternalError`, because this engine cannot translate that text.

Both modules have unit tests (`#[cfg(test)] mod tests`).

## Official API names mirrored

| Framework | BifrOSt |
|---|---|
| `LanguageAvailability`, `status(from:to:)`, `supportedLanguages` | `LanguageAvailability::status`, `supported_languages` |
| `LanguageAvailability.Status` (installed, supported, unsupported) | `AvailabilityStatus` |
| `TranslationError` cases, `errorDescription`, `failureReason` | `TranslationError`, `error_description`, `failure_reason` |
| `TranslationSession(installedSource:target:)` | `TranslationSession::new` |
| `translate(_:)`, `translations(from:)` | `translate`, `translate_batch` |
| `cancel()` | `cancel` |
| `sourceLanguage`, `targetLanguage` | `source_language`, `target_language` |

## Official documentation and REA evidence

- `python3 -I tools/parity/docc.py translation` (and `translation/translationsession`,
  `translation/languageavailability`, `translation/languageavailability/status`, `translation/translationerror`)
  gave the symbol lists, the availability statements and the case names used above.
- REA evidence of the rendered landing page: `ev_d4f5aa8667ccac14efa9072bd392c2b8b1de199cedcebba1ed78b61e74a3934b`
  (title "Translation | Apple Developer Documentation").
- Earlier REA evidence recorded by the matrix: `ev_318b6155181a029fffc215b27f3997bb92800b4751200f5d8b7a005c5a848088`.

## Remaining gaps

- No on-device translation engine or language packs. The session only runs over an engine the app supplies.
- No system translation popover (`translationPresentation`) and no inline translate action.
- No `translationTask` equivalent.
- No `status(for:to:)` from sample text, so no language identification and no `unableToIdentifyLanguage`.
- No `prepareTranslation()`, `canRequestDownloads` or download request.
- No `TranslationSession.Configuration` or `Strategy`.
- No formatted-text translation (`AttributedString` input).
- The supported language list is not built in. The caller supplies it.
- Locale data covers only `en` and `is`.

## Verification

Not built or run in this stage, as the task requires. The next stage builds and tests the crates.
Owner-rule check: `python3 tools/check-names.py` exits 0 with no hits.
