# Wire 3: System text services (spelling, text replacements)

Area: a shell service that apps call for spelling checks, learned and ignored words and text replacements, an SDK
client for it, and the Settings > Keyboard > Text Replacements list. Branch `wire3/text-services`.

## What is now wired

- **Spelling checker in the shell.** The checker model moved from `springboard/sdk/src/spellcheck.rs` to
  `springboard/core/src/spellcheck.rs` (`bifrost_core::spellcheck`) so the shell can use it. The shell cannot depend on
  the SDK crate (the SDK depends on the shell crate). The SDK file now re-exports the core model (`bifrost_app::spellcheck`
  keeps every name) and implements `KeyValues` for `UserDefaults`. Behaviour is unchanged.
- **Persistence seam.** `SpellChecker::load`, `learn_word` and `unlearn_word` take `&impl KeyValues` instead of
  `&UserDefaults`. The SDK implements it for `UserDefaults`; the shell implements it for its settings file
  (`Prefs`), storing the learned words one line each joined by spaces, since settings values are single lines.
- **Replacement list model.** `springboard/core/src/text_replacements.rs` (`bifrost_core::text_replacements`):
  `Replacements` with `set` (add or change, trimmed, refusals for an empty shortcut or phrase, a shortcut with a
  space, or control characters), `remove`, `expand` (exact whole-word match), a sorted `entries()` and a one-line
  saved form (`to_line` / `from_line`).
- **Shell service `text`** (`springboard/src/shell/services/text_services.rs`, registered in `services::all()`).
  The checker starts from the keyboard's English word list (`apps/keyboard/src/words/en.txt`, 20,000 words, the same
  list the on-screen keyboard predicts from). Methods: `check_spelling`, `check`, `correction`, `guesses`,
  `learn_word`, `unlearn_word`, `has_learned_word`, `ignore_word`, `replacements`, `set_replacement`,
  `remove_replacement`, `expand`. Errors are returned as messages, never panics. The state is loaded at shell start
  (`services::text_services::load`, one call in `shell/mod.rs`).
- **SDK client** (`springboard/sdk/src/text_services.rs`, `bifrost_app::text_services`): one function per method. Each
  returns the request id; the reply arrives as `App::service_reply` (or `service::take_reply`).
- **Settings > Keyboard > Text Replacements** (`pages.rs`, `settings.rs`, keyboard entries only). The Keyboard page
  gets a row "Text Replacements" with the count (or "None"). The page lists each `shortcut` with its phrase. Tapping
  an entry asks "Remove Text Replacement?" (destructive confirm) and removes it from the saved list. An empty list shows
  a dim "No Text Replacements" row.

## How a user reaches it

Settings > General > Keyboard > Text Replacements. Removing an entry: tap it, then Remove in the confirm alert.
Apps reach the checker through `bifrost_app::text_services` (asynchronous: the reply is delivered later).

## How to verify on Midget

1. `bifrost pc midget state` (the shell is up), then open Settings and go to General > Keyboard. The row reads
   "None" on a fresh device.
2. Tap Text Replacements. It shows one dim row "No Text Replacements".
3. To seed an entry, on Midget (`ssh midget`), edit `/var/lib/springboard/settings.ini` and add the line
   `text.replacements=omw<US>On my way` (`<US>` is the control character U+001F, for example written with
   `printf '\037'`). Restart the shell (`bifrost pc midget key enter` only unlocks; restart via Settings > General >
   Restart BifrOSt). The Keyboard row reads "1", and the page lists `omw` with "On my way".
4. Tap `omw`, confirm Remove. The entry is gone from the page and from `settings.ini` (the file is rewritten by the
   shell).
5. Screenshot the Keyboard and Text Replacements pages (`bifrost pc midget screen PATH`) and check them against the
   bifrost-hig checklist (LAY, TYP, COL, CTL).
6. Host checks: `cargo test -p bifrost-core spellcheck text_replacements`, `cargo test -p bifrost-app text_services`
   (SDK client), and `cargo test -p springboard text_services keyboard_links` (shell service and page). These were not
   run in this round; the lead builds once.

## REA evidence

- NSSpellChecker (official page, rendered): `ev_9aa390e37be0352e7c4d00b20a418972e45abcf23cfaf1a25b218d547e828a36`
  (https://developer.apple.com/documentation/appkit/nsspellchecker). Covers `checkSpelling(of:startingAt:)`, `guesses`,
  `learnWord`/`unlearnWord`/`hasLearnedWord`, `ignoreWord`, and the `userReplacementsDictionary` entry point.
- userReplacementsDictionary (rendered): `ev_215169b45e53c4752b69c870f416e092b91f6b72e2347785539a198175893af0`
  (https://developer.apple.com/documentation/appkit/nsspellchecker/userreplacementsdictionary). Replacements are a
  key-value dictionary used when replacing words.
- completions(forPartialWordRange:in:language:inSpellDocumentWithTag:) (rendered): `ev_41b39539e3e74d1272003916fc275ddfe0ef02aaaefe47abf3bf8f283e2c0c5a`
  (https://developer.apple.com/documentation/appkit/nsspellchecker/completions(forpartialwordrange:in:language:inspelldocumentwithtag:)).
  Word completion; not wired in this round (see gaps).

## Remaining gaps

- **No add sheet.** A replacement can be set by an app through `set_replacement`, and removed in Settings, but the
  page has no "Add" row yet (it needs a text entry sheet, which the Keyboard page does not have).
- **Replacements are not applied while typing.** The on-screen keyboard and text views do not call `expand` yet; the
  page footer says so. Apple's replacement dictionary also applies to typed text in every app.
- **No grammar checking.** There is no grammar data; `check` returns spelling results only.
- **No text completion.** `completions(forPartialWordRange:...)` has no model or service method.
- **No Settings toggles** for "Check Spelling While Typing" or "Correct Spelling Automatically" (the Keyboard options
  need new `Options` fields in `options.rs`, which this wave does not own).
- **No learned-word or ignored-word list in Settings.** Learned words are only added by apps.
- **Writing Tools** (`ui/src/writing_assist.rs`) is not connected to the service.

## Matrix rows

| Slug | Feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| appkit-text | Text Display: spell checking | springboard/core/src/spellcheck.rs (model), springboard/sdk/src/spellcheck.rs (SDK re-export), springboard/src/shell/services/text_services.rs (service) |
| natural-language | Spelling-aware word lookup (companion, not Apple NL) | springboard/core/src/spellcheck.rs (`words`, `WordList`, `check_spelling`), springboard/src/shell/services/text_services.rs |

Text replacements have no matrix row. Their model is springboard/core/src/text_replacements.rs. The matrix data is
not edited in this round, as the wave rules require.
