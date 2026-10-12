# Wave 4: Core ML (on-device model updates)

## What was built

- `springboard/ui/src/ml_update.rs` (`bifrost_ui::ml_update`): a pure, synchronous stochastic gradient descent
  step that personalizes a `Model` (`springboard/ui/src/ml_model.rs`) from labelled samples.
  - The trainable layer is the model's last dense layer. Layers before it are frozen and run once per sample.
  - The loss is softmax cross-entropy over that layer's outputs. The gradient is `softmax(logits) - one_hot(label)`.
  - `update(&model, &samples, &Parameters)` returns `(Model, Report)`. The input model is not changed.
  - `Parameters { learning_rate, epochs }` is the optimizer's rate and epoch count. `Report` gives the mean
    loss before and after training.
  - Errors: `UpdateError::{Empty, Epochs, LearningRate, NoDenseLayer, InputLength, Label, Model}`.
  - Sample order is fixed, so the result is deterministic.
- `springboard/ui/src/lib.rs`: one `pub mod ml_update;` line.

## Official API mirrored

- `MLUpdateTask` (https://developer.apple.com/documentation/coreml/mlupdatetask): the update of a model from
  training data. The mirror is `update`. It runs synchronously, not as a task with completion or progress
  handlers.
- `MLParameterKey.learningRate` and `MLParameterKey.epochs`
  (https://developer.apple.com/documentation/coreml/mlparameterkey): the mirror is `Parameters`.
- `MLBatchProvider`: the mirror is a plain `&[Sample]` slice. No batch provider type exists.

## REA evidence

- `ev_33fabba814e1d90ce80b1d49f6b98e115d44464ed85b9a9ddd93ec25ee5a6407`: rendered MLUpdateTask page
  (title "MLUpdateTask | Apple Developer Documentation"). Captured with `tools/parity/rea-page.sh`.

## Not built, and why

- Custom layers and custom models: the model has no extension point. Adding a custom layer would mean a trait
  object in `Layer`, which breaks its derived `PartialEq`. Not attempted in this wave.
- Model encryption: needs a real cipher. The rules allow no new crates and no unsafe code, so a hand-written
  cipher is not an option.
- Downloading and compiling a model at runtime: needs a model distribution service that BifrOSt does not have.
- Domain frameworks built on the Core ML overview: out of scope for this wave.

## Gaps left in the on-device update row

- No SDK API. Apps cannot call `update` yet; `bifrost_app` does not re-export it.
- No training data from disk or a database. Samples must be built in memory.
- No updated model file is written back. The result is only returned to the caller.
- No background execution, cancellation or progress reporting.
- Updates train only the last dense layer. Hidden dense layers, other losses, momentum, mini-batches and the
  Adam optimizer are not built.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/core-ml.json`) | New status | BifrOSt path |
|---|---|---|---|
| core-ml | On-device model updates | partial | springboard/ui/src/ml_update.rs |

## Verification

- The tests in `ml_update.rs` cover one hand-worked SGD step (weights [0.5,-0.5,0,0], bias [0.5,-0.5]), the
  loss values ln 2 = 0.6931 before and 0.1269 after one step, a second epoch (about 0.0807), frozen earlier layers,
  rejected samples and parameters, and a model with no dense layer.
- `python3 tools/check-names.py` was run on the tree; it printed no hits.
- No cargo build or test run was made, as the wave rules require.
