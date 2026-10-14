# Parity sdk-complete-25: analysis runtime, sentiment, vector text, result log

Scope: the partial rows for media intelligence, core ML domain frameworks, contextual natural language embeddings and
sound analysis results.

## What was added

- `springboard/ui/src/ml_runtime.rs` (`AnalysisRuntime`): one table of named models that every analysis request shares.
  Models are loaded once under a name, requests are routed by name, refused names are reported, and requests are
  counted per model. It mirrors the shared on-device analysis service that many clients call, with no compute device.
- `TextClassifier::sentiment` in `springboard/ui/src/text_classifier.rs`: a two-label positive and negative preset built
  from word lists. The lists are the model; no weights are trained.
- `Embedding::from_vector_text` in `springboard/ui/src/embedding.rs`: loads a vocabulary from text lines (a term, then its
  components). It is the loader the contextual natural language asset path needs before a binary reader exists.
- `ResultLog` in `springboard/ui/src/sound_analyzer.rs`: a results observer whose records stay readable after the analyzer
  takes ownership of the observer. Clones share the records.

Tests: `springboard/ui/tests/parity_sdk-complete-25.rs`.

## What is wired

Nothing in the shell or in apps calls these types yet. The types are pure models and run on the host.

## What needs the lab machine

Live audio capture for the sound classification sample, a model served to a real app, and any compute device acceleration
are not available here. They need Midget with an audio input device and a deployed app.
