# Wire 2: Assistant (guardrail, skill routing, diagnostics evaluation)

Branch `wire2/assistant`, Assistant 1.2.0. Three wave-6 pure models are now used by the Assistant app
(`springboard/apps/assistant-desktop/`). The models themselves are unchanged; the app adds a small glue module,
`src/safety.rs`, and wires it into the chat window (`src/view.rs`), the request code (`src/api.rs`) and the Chat menu
(`src/main.rs`).

## What is wired

| Model | How the app uses it |
|---|---|
| `bifrost_app::controls::guardrail::Guardrail` (`springboard/ui/src/guardrail.rs`) | Every answer is screened before it is shown or saved. Streaming text is screened as it grows: a blocked phrase is masked once whole, and the tail that could still become one is held back. A finished answer over 20,000 characters is cut with a note. Blocked phrases come from `guardrail.txt` in the app data folder (one per line; `#` lines are notes). |
| `bifrost_app::controls::text_classifier::TextClassifier` (`springboard/ui/src/text_classifier.rs`) | Routes each request to a skill by the keywords in it. The weights are hand-set (one per keyword for its own label), so the classifier is a keyword router, not a trained model. A matched skill sends its system prompt with the request (`"system"` in the Messages body). |
| `bifrost_app::controls::evaluation::{Evaluation, Evaluator, Sample, MetricsAggregator, model_judge, ModelJudgePrompt, ScoreDimension}` (`springboard/ui/src/evaluation.rs`) | Chat > Diagnostics runs an `Evaluation` over the newest 10 exchanges of the open chat. Rule checks: "Not Empty", "Passes Guardrail". With an API key, a model judge ("Model Judge") rates each answer on Helpfulness and Clarity (1 to 5, normalised to 0..1) by sending the rubric prompt to the same model. The report gives mean, median and range per metric and says how many answers were not scored. |

Skill labels and their keywords (`safety.rs`, `LABELS` and `KEYWORDS`):

- Code: code, function, bug, compile, rust, python, program, implement, debug, error, script
- Writing: write, email, letter, essay, story, poem, draft, rewrite, paragraph
- Summary: summarize, summary, tldr, shorten, bullet, brief
- Translation: translate, translation, icelandic, english, danish, german

A request with none of these words goes out with no system prompt, as before.

## How a user reaches it

- Answers: automatic. Nothing to set. A blocked-phrase list is optional: create `guardrail.txt` in the Assistant data folder
  (the folder that holds `conversations.json`).
- Skill routing: automatic on every send. The sheet shows the last route.
- Diagnostics: Chat > Diagnostics... (Cmd+Shift+D). The sheet shows the guardrail settings, the blocked phrase count,
  the last route, and the report. Run Evaluation starts the run on a background thread; Return runs it too; Close or
  Escape closes the sheet; clicking outside the panel closes it.

The only menu change is one Diagnostics item in the existing Chat menu; the app still has one menu, titled Chat.

## How to verify on Midget

1. Build the app (the lead builds once) and publish it through the usual flow (`bifrost pc publish`, then
   `bifrost pc midget ota` until `phase=accepted`).
2. Open Assistant on Midget (`bifrost pc midget open-app is.olibuijr.assistant.desktop`), then take a screenshot
   with `bifrost pc midget screen /tmp/assistant.png`.
3. Guardrail: put `secret phrase` on its own line in `guardrail.txt` (data folder), ask the model to say "secret
   phrase". The answer must show `[removed]` in its place, and no part of the phrase must appear while it streams.
4. Routing: send "fix this rust bug" and "translate this to Icelandic". Open Chat > Diagnostics and check "Last request
   routed to: Code" and then "Translation".
5. Diagnostics: with an API key set, press Run Evaluation after two or three exchanges. Expect "Not Empty", "Passes
   Guardrail" and "Model Judge" lines with mean, median, range and the count. Without a key, the sheet says to add
   one and only the two rule checks report.
6. Dark and Light: `bifrost pc midget appearance light` and `dark`, then a screenshot of the sheet; the sheet uses the
   same tokens as Settings (`CARD_BG`, `FIELD_BG`, `LABEL`, `SYSTEM_BLUE`).

Status stays `partial` in the matrix until steps 3 to 6 pass on Midget (owner rule: a row is `parity` only after a
Midget test).

## REA evidence

Official documentation and the matrix evidence ids for these rows were taken from `docs/parity-matrix/data/`. They were
not re-captured in this round (`docc.py` and `rea-page.sh` were not run for this wire; no new evidence was captured).

- `ev_ab0c276e8f08660eff146ce88e3a628c67e0301757f4d384a0819607760d8663` (Foundation Models, guardrail row, `foundation-models.json`)
- `ev_756075273a0ac335520262738baf7c9059ec686f84b2fc7f27b211c0e86a0e9b` (Evaluations, `evaluations.json`)
- `ev_9e68febdd223107558efdf4b30d4323aefbf32ccac1dce92a912c41429217f7b` (Natural Language, `natural-language.json`)

## Remaining gaps

- The result table (`EvaluationResult`, `ResultColumn`, `inputColumn`, `responseColumn`, `expectedColumn`) is not
  wired: the sheet shows report lines, not a table with a column per field.
- Datasets and sample generation (`ModelSample`, `Loader`, `SampleGenerator`): samples come from the chat's own
  history only; no dataset file loader and no synthetic generator.
- Tool-call trajectory evaluation: not wired (no tool use in the Assistant yet).
- The text classifier is keyword-weighted. There is no trained text model, no model file format and no word tagger,
  so the "Custom text models" row stays partial.
- The guardrail is a phrase list and a length limit. It cannot judge meaning, and the phrase list has no UI yet
  (it is a file). The SDK still has no output filter for other apps.
- The judge uses the same model as the chat, so it shares that model's biases. The judge makes one request per answer
  and dimension (up to 20 requests per run for 10 answers); this is user-initiated only.
- Not verified on the test PC yet (see above); host unit tests in `src/safety.rs` and `src/api.rs` cover the pure
  parts (screening, routing, exchanges, report lines, request body).

## Matrix rows

| slug | feature (as written in `docs/parity-matrix/data/<slug>.json`) | BifrOSt path |
|---|---|---|
| foundation-models | Safety guardrails for generated output | springboard/ui/src/guardrail.rs (model); springboard/apps/assistant-desktop/src/safety.rs, src/view.rs (wired) |
| evaluations | Evaluation definition and run | springboard/ui/src/evaluation.rs (model); springboard/apps/assistant-desktop/src/safety.rs (`run_evaluation`), src/view.rs (Diagnostics) |
| evaluations | Metrics, evaluators and aggregation | springboard/ui/src/evaluation.rs (model); springboard/apps/assistant-desktop/src/safety.rs (`report_lines`) |
| evaluations | Model-judge evaluators | springboard/ui/src/evaluation.rs (model); springboard/apps/assistant-desktop/src/safety.rs (`judge_call`, `JUDGE_TEMPLATE`) |
| natural-language | Custom text models (NLModel, Create ML text classifier and word tagger) | springboard/ui/src/text_classifier.rs (model); springboard/apps/assistant-desktop/src/safety.rs (`router`, `route`) |
