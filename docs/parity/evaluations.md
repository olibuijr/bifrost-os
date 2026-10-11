# Evaluations: parity note

Framework: Evaluations (https://developer.apple.com/documentation/evaluations). Status moves from gap to partial.

## What was built

`springboard/ui/src/evaluation.rs` (bifrost-ui, pure std, no I/O, no model). It is registered in `springboard/ui/src/lib.rs`.

- Datasets: `Sample` (input and optional expected value).
- Run: `Evaluation::run(samples, responder)` answers each sample through a caller-supplied responder and scores it with every evaluator. A failed response is recorded as a `RunError` and leaves no row.
- Evaluators: `Evaluator::new` takes a closure over the sample and response. `Evaluator::check` scores 1.0 or 0.0, so a metric's mean is a pass rate.
- Aggregation: `MetricsAggregator::summarize` gives count, mean, median, min, max, population variance and standard deviation. `EvaluationResult::summary` does this for each metric.
- Results: `EvaluationResult` holds rows and errors, `metric_values(name)`, and `column(&Column)` for input, response, expected or one metric.
- Tool calls: `ToolCall`, `ToolExpectation` and `TrajectoryExpectation::check`. Ordered steps are matched greedily, each at the earliest call after the previous step. Unordered calls may appear anywhere. Disallowed names fail. Extra calls fail unless `allowing_additional_calls()` is set.
- Argument rules: `ArgumentMatcher` covers exact, key-only, one-of, numeric range (inclusive), contains, prefix and suffix. A missing argument never matches.

## Official API names mirrored

Evaluation, ModelSample (as Sample), Metric, Evaluator, MetricsAggregator, EvaluationResult, ResultColumn (as Column), inputColumn, responseColumn, expectedColumn, ToolCallEvaluator (as TrajectoryExpectation::check), TrajectoryExpectation, ArgumentMatcher (exact, keyOnly, oneOf, range, contains, hasPrefix, hasSuffix).

Official API shapes were read with `python3 -I tools/parity/docc.py` for `evaluations`, `evaluations/metricsaggregator`, `evaluations/argumentmatcher` and `evaluations/trajectoryexpectation`.

## REA evidence

- `ev_aed8a863ae3b41881c1d29174477654902d82c201ec038884b31cbf145e8840e`: rendered official landing page (title "Evaluations | Apple Developer Documentation").

## Remaining gaps

- The Assistant app does not run an evaluation suite. `springboard/apps/assistant-desktop/src/model.rs` is unchanged.
- No Loader protocol and no SampleGenerator. Datasets are built in code.
- No model-judge scoring (ModelJudgeEvaluator, ModelJudgePrompt, ScoreDimension).
- ArgumentMatcher has no regex or natural-language rule, and MetricsAggregator has no mode, custom or grouped aggregation.
- The result table is not a typed DataFrame.
- No EvaluationTrait for `springboard/sdk/src/testing.rs`.

## Not verified

No cargo build or test was run in this stage, per the task. The unit tests in `evaluation.rs` (7 tests) are written but not executed.
