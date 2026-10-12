# Wave 3, theme 27: domain frameworks built on Core ML

Branch `parity/w3-theme-27`. Theme: the gap rows around the on-device text, speech, transform and dictionary
frameworks. This round builds four pure models, one per gap row that a model can honestly cover. Nothing was compiled
or run (the wave rule is no builds in worktrees); the unit tests were worked out by hand from the rules each module
implements and are for the integration build to confirm.

## What was built

| Module | Crate | Mirrors (official API) | What it does |
|---|---|---|---|
| `springboard/ui/src/fourier.rs` | bifrost-ui | Accelerate vDSP Fourier transforms and discrete cosine transforms: `vDSP` (Fourier and cosine functions), "Performing Fourier transforms on interleaved-complex data" | In-place radix-2 `fft` and scaled `ifft` for power-of-two lengths, a direct `dft` for any length, unnormalized DCT-II `dct2` and its inverse `idct2`. `f64`, with a `FourierError` enum. |
| `springboard/ui/src/embedding.rs` | bifrost-ui | Natural Language `NLEmbedding`: `neighbors(for:maximumCount:distanceType:)`, `distance(between:and:distanceType:)`, `vector(for:)`, `contains(_:)`, `dimension`, `vocabularySize`, and the sentence embedding idea | A caller-supplied vocabulary of equal-length vectors. `DistanceType::Cosine` and `Euclidean`. Nearest terms to a term or to a vector, excluding the query term. Sentence vector as the mean of the known words. No model file reader and no bundled vectors. |
| `springboard/ui/src/lexical_tags.rs` | bifrost-ui | Natural Language `NLTagger` with the lexical class scheme (`NLTag` lexical class: noun, verb, adjective, adverb, pronoun, determiner, preposition, conjunction, interjection, number, punctuation, whitespace, other word) | Tokenizer with byte ranges, a closed-class lexicon, a number rule, punctuation per character, and suffix rules for longer words. Rule-based, so the classes are guesses where the suffix rules apply. |
| `springboard/sdk/src/definitions.rs` | bifrost_app | Core Services dictionary lookup: `DCSCopyTextDefinition` (definition of a term) and `DCSGetTermRangeInString` (range of the term at an offset) | `DefinitionIndex` stores definitions keyed by normalized term, finds a definition for a term, the longest stored term covering a byte offset in a text (multi-word terms allowed across whitespace only), and both together. Reuses `spellcheck::words` for word boundaries. No bundled dictionary. |

The four are pure models with `#[cfg(test)] mod tests` blocks (five or more behaviour tests each). Each module has one
`pub mod` line in its crate's `lib.rs`. `python3 tools/check-names.py` exits 0.

## Official references

- Accelerate landing page: https://developer.apple.com/documentation/accelerate (REA `ev_aed25292ca69d385852550a8bb76d2400b23c29efe68ee91a8c5a7b7551fe80b`). DocC topic "Fourier and Cosine Transforms" and "Neural Networks" (vDSP, BNNS) read with `docc.py accelerate`.
- Natural Language landing page: https://developer.apple.com/documentation/naturallanguage (REA `ev_d3bee70d360d7396d3ac06398ef9f6aa01fb6a5ff37dcc13b214dd246126467f`). DocC `naturallanguage/nlembedding` (creating a word embedding, finding strings and their distances) and `naturallanguage/nltagger` (enumerating linguistic tags) read with `docc.py`.
- Core Services landing page: https://developer.apple.com/documentation/coreservices (REA `ev_0eaf7025e15cb5ea7015fa2d3c8d46d1ec28ba81c3dfd84dc87f4d2f2ea663f5`). DocC topic "Dictionary Services" ("Look for words and phrases in system dictionaries") read with `docc.py coreservices`.
- The REA page for `https://developer.apple.com/documentation/natural-language/nlembedding` returned a "page not found" render (`ev_724fa5b0384567909734f5efeb11215f70eaa8eaa7bd3e48dc3af49ee5ebf927`), and the DocC path `naturallanguage` is the correct one. The DocC paths `natural-language/...` and `coreservices/...` (with hyphen) returned HTTP 404; the right paths are `naturallanguage/...`.

## Rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| accelerate | FFT, DFT and DCT | partial | springboard/ui/src/fourier.rs |
| natural-language | Text embedding and similarity (NLEmbedding) | partial | springboard/ui/src/embedding.rs |
| natural-language | Parts of speech and lexical tagging (NLTagger, NLTag lexical class) | partial | springboard/ui/src/lexical_tags.rs |
| core-services | Dictionary lookup | partial | springboard/sdk/src/definitions.rs |

All four are `partial`: none is wired into an app or the shell, none has been run on the VM or Midget, and each leaves
out the main data side (no trained embedding, no lemma or inflection data, no bundled dictionary, no real-input FFT
packing or batch calls).

## Not closed (and why)

- Natural Language "Custom text models" (`NLModel`, Create ML text classifier): needs a trained model format; out of
  scope for a pure model with no data.
- Accelerate "Neural networks" (BNNS): `springboard/ui/src/nn.rs` already exists; a second inference path would
  duplicate it, so the row is left as it was.
- Core Image "Feature detection" (`CIDetector`): needs image analysis of pixels, not a text or transform model.
- Speech and SiriKit rows: the speech models already exist from wave 2 (`recognition.rs`, `voice_activity.rs`); no
  recognition engine is in this round.

## Tests (to be confirmed by the integration build)

- `fourier.rs`: impulse and constant inputs for `fft`; the hand result for `[1, 2, 3, 4]` (`X0 = 10`, `X1 = -2 + 2i`,
  `X2 = -2`, `X3 = -2 - 2i`); `ifft` round trip; error cases (length 3, empty, mismatched imaginary part); `dft` of
  `[1, 0, 0]` and agreement with `fft` on length 4; `dct2` of a constant and of an impulse (`X[k] = cos(pi k / 8)`),
  with `idct2` inverting it.
- `embedding.rs`: term normalization and dimension checks; Euclidean distance 5 for `(0, 0)` to `(3, 4)`; cosine
  distance 1 for orthogonal, 0 for the same direction, 1 for a zero vector; neighbours of `cat` by cosine (`bird` at
  `1 - 1/sqrt(2)`, then `dog` at 1); neighbours of `[1, 1]` by Euclidean; the sentence vector `(2, 0)` from the mean of
  `(1, 0)` and `(3, 0)`, and `None` with no known word.
- `lexical_tags.rs`: a whole sentence's token classes; byte ranges (including a two-byte first letter); suffix rules
  and the length cutoff; per-character punctuation; the words helper.
- `definitions.rs`: case-insensitive storage and lookup, empty term error; single word range `7..11`; longest term
  `4..17` covering offsets 5 and 10; no term in whitespace; `definition_at` returning range and definition.

## Commands run

- `python3 tools/check-names.py`: exit 0.
- `tools/parity/rea-page.sh <url>` for the three landing pages and the two sub-pages above.
- `python3 -I tools/parity/docc.py naturallanguage/nlembedding`, `naturallanguage/nltagger`, `accelerate`, `coreservices`.
- No cargo, rustc, make or test runs.
