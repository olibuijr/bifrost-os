# Theme 11: Errors

Wave 2 work for the parity matrix theme "Errors" (34 rows, 23 frameworks). Most rows are error enums of frameworks
BifrOSt does not have (CloudKit, EventKit, Core Location, Virtualization and others), so they stay gaps. This round
builds the shared part they would all use, and applies it to the three rows that have a BifrOSt counterpart.

## What was built

- `springboard/sdk/src/error.rs`: the `ErrorCode` trait. A typed error has a domain constant, a numeric code (declaration
  order from 1), a case name, a localized description, and `to_app_error()`, which returns an `AppError` with the
  same domain and code. Apps test a failure with `AppError::is(domain, code)` instead of matching text.
- `springboard/sdk/src/accounts.rs`: `ACCOUNT_DOMAIN` (`bifrost.accounts`) and `ErrorCode` for `AccountError`
  (codes 1 to 7). `Display` now reads the same description, so the text is in one place.
- `springboard/sdk/src/translation.rs`: `TRANSLATION_DOMAIN` (`bifrost.translation`) and `ErrorCode` for
  `bifrost_ui::translation::TranslationError` (codes 1 to 8). The eight cases match the official list one for one.
- Tests (in `error.rs`, which holds the trait): the typed error becomes an `AppError` with its domain, code and text;
  codes are unique and start at 1; the translation domain and codes; plus the existing `AppError` tests.

Scope limits, stated so they are not read as more than they are:

- The domain strings are BifrOSt's own (`bifrost.*`), like the existing `bifrost.url`. Apple's numeric raw values are
  not in the official pages we read, so the codes are BifrOSt's declaration-order codes, not Apple's raw values.
- `AccountError` models 7 of the 23 official `ACErrorCode` cases. The store never produces the other 16 (keychain,
  protection policy, plug-in and transport failures), so they are not added. The row stays partial.
- The `AppError` gap list is unchanged: no per-domain user info providers and no recovery attempter.
- No builds or test runs were done in this round, per the wave rules.

## Official API names mirrored

- `ACErrorCode` (accounts), `ACErrorDomain`; `TranslationError` (translation) with `nothingToTranslate`,
  `unableToIdentifyLanguage`, `internalError`, `alreadyCancelled`, `notInstalled`, `unsupportedSourceLanguage`,
  `unsupportedTargetLanguage`, `unsupportedLanguagePairing`, `errorDescription`.
- `NSError` error domain and code model (foundation), via the existing `AppError`.

## REA evidence

- `ACErrorCode`: `ev_58c72636d5a811ebc1ffef8a9162e138c0ad48f1d832e70a6db8cae5b613d7ac`
  (https://developer.apple.com/documentation/accounts/acerrorcode)
- `TranslationError`: `ev_79cf0f0e4b5c584e6db0d9030a60db21a7b8f797fd11a64135da1872e4cbd02e`
  (https://developer.apple.com/documentation/translation/translationerror)

## Matrix rows closed

| Framework slug | Feature (as in data/<slug>.json) | New status | BifrOSt path |
|---|---|---|---|
| accounts | Errors (ACErrorCode, ACErrorDomain) | partial | springboard/sdk/src/accounts.rs (AccountError, ACCOUNT_DOMAIN, ErrorCode) |
| translation | Translation errors | parity | springboard/sdk/src/translation.rs (TRANSLATION_DOMAIN, ErrorCode for TranslationError) |
| foundation-app-support | Errors and Exceptions | partial | springboard/sdk/src/error.rs (ErrorCode trait on AppError) |

Rows left as gap or n/a, with no BifrOSt counterpart yet: cloudkit, eventkit, cryptotokenkit, core-location, callkit,
service-management, virtualization, sound-analysis, link-presentation, tipkit, storekit, statereporting,
accessory-access, media-intelligence, gamesave, exception-handling, avrouting, lightweightcoderequirements,
javascriptcore, tabular-data.
