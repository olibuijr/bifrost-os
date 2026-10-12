# Wave 4, theme 24: formatted-text translation, Braille, run delegates

Theme: "Formatted-text translation" (size 4, frameworks Translation, Accessibility, Core Text). Three pure models
were added or extended. Nothing here is wired into the shell or the apps yet, and nothing has been built or run on
the VM or the Midget (the wave ran without builds).

## What was built

1. **Formatted-text translation** (`springboard/ui/src/translation.rs`, `translate_runs`; `springboard/sdk/src/translation.rs`,
   `TranslationSession::translate_attributed`). The runs of an attributed string are joined with a private-use
   separator (U+E000), one engine call translates the whole text, and the separators in the result mark the new run
   boundaries. The n-th translated piece takes the attributes of the n-th run, so bold, italic, colour and links
   survive. An engine that drops or adds separators, or text that already holds U+E000, fails with `InternalError`.
   Blank text fails with `NothingToTranslate` before the engine runs, and `cancel()` applies as for plain text.
   The engine is still the app's own `Translator`; BifrOSt has no language packs, so no real translation runs yet.

2. **Braille translation** (`springboard/ui/src/braille.rs`, new). Text to six-dot cells (Unicode Braille Patterns
   order, bit 0 is dot 1). Grade 1 letters a to z, digits with the numeric sign (a to j, one sign per number), the
   capital sign before each capital letter, the common punctuation marks, parentheses and the double quote as
   two-cell marks. Characters outside that set fail with `BrailleError::UnsupportedCharacter`. `Display` is a
   fixed-width window over a cell line with pan and page count, the model a display driver needs. There is no
   device protocol and no contraction table (Grade 2 is not built).

3. **Run delegates and adaptive images** (`springboard/ui/src/run_delegate.rs`, new). `RunMetrics` (ascent, descent,
   width, clamped to zero) is what a run delegate reports. `Piece` is a text run or an attachment with an owner id.
   `line_metrics` takes the largest ascent and descent and the sum of widths; `attachment_at` finds the attachment under
   a horizontal position. `pick_variant` chooses an adaptive image variant by display scale and appearance (the
   smallest scale at or above the display, dark or light first), and `fit_size` aspect-fits an image into a proposed
   size.

## Official API names mirrored

- `TranslationSession.translate(_:)` for an attributed string, "Translates a formatted string of text, preserving
  formatting in the translation" (Translation, `translationsession`). REA: `ev_0bd0720822272805b398c043e3c31c5a1203eaacfb0eb550db37009959b09164`.
- `CTRunDelegate` callbacks for ascent, descent and width, and the line metrics they feed (Core Text,
  `ctrundelegate`). REA: `ev_90334cb536498ba7beb1b4db02d8fc01c02a672225f396c0faa6abd7d816d3a4`.
- `CTAdaptiveImageProviding.image(forProposedSize:scaleFactor:imageOffset:imageSize:)`, an image chosen for a
  proposed size and scale (Core Text, `ctadaptiveimageproviding`). REA: `ev_1f3b14d012f53d83ac21721baa95e9b9b445806193386141e975773fa8c400ea`.
- Braille translation has no official API page that the DocC accessibility index lists (checked with
  `docc.py accessibility`; no hit). The Braille model follows the Grade 1 cell rules as a concept and cites no
  Apple API name for it. No REA evidence exists for it.

## Matrix rows closed

| Framework slug | Feature (as in data file) | New status | BifrOSt path |
|---|---|---|---|
| translation | Formatted-text translation | partial | springboard/ui/src/translation.rs (`translate_runs`), springboard/sdk/src/translation.rs (`TranslationSession::translate_attributed`) |
| accessibility | Braille displays and Braille translation | partial | springboard/ui/src/braille.rs (translation and display window; no device driver) |
| core-text | Run delegates and embedded attachments | partial | springboard/ui/src/run_delegate.rs (metrics, line metrics, hit test, adaptive image choice) |

Why `partial` and not `parity`:

- Translation: the formatted overload exists, but the engine is app-supplied and BifrOSt ships no translation
  backend. The popover row ("System translation popover") is untouched and stays a gap.
- Braille: no Braille display hardware or protocol, and no Grade 2 contractions. The display row is a model only.
- Run delegates: the metrics and the hit test are pure. Attachments are not yet an attribute of `AttributedString`
  and the renderer does not lay out or draw them, so an app still cannot place an image inside styled text.

## Tests

Each changed module has tests with values worked out by hand: `run_tests` in `ui/src/translation.rs` (6), the
sdk `translation.rs` tests (2 new), `ui/src/braille.rs` tests (7), and `ui/src/run_delegate.rs` tests (6). Nothing was
compiled or run in this wave, as the wave rules require.
