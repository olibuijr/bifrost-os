# Wave 3: Core ML (gap rows)

Two gap rows of the Core ML matrix are now partial: a model runner and state buffers for stateful models.
Both are pure Rust, std only, with no hardware, no network and no unsafe code.

## What was built

- `springboard/ui/src/ml_model.rs` (bifrost-ui): a model is a named tensor input, a chain of layers (dense, relu,
  softmax) and a named output. `Model::predict` checks the input provider with `features::check`, rejects an input
  whose element count differs from the declared length, runs the layers on the CPU and returns the output as a
  `DictionaryProvider`. `Model::parse` reads a small text form (`input`, `dense`, `relu`, `softmax`, `output`),
  and reports the 1-based line of any bad line. This is an evaluator, not a loader for compiled model packages.
- `springboard/ui/src/ml_state.rs` (bifrost-ui): named state buffers with a shape constraint each.
  `ModelState::new` creates zeroed buffers and refuses duplicate names. `read`, `write`, `add` (element-wise
  running total) and `reset` check names and shapes. The buffers are not yet passed into `Model::predict`, so a
  recurrent model cannot use them until the runner grows a stateful layer. That is the next step.
- `springboard/ui/src/lib.rs`: one `pub mod` line for each new module.

Both modules reuse the existing wave 1 types: `features` (providers, feature kinds) and `ml_tensor` (tensors).
No module was duplicated.

## Official API mirrored

- `MLModel`, `MLModelAsset`, `MLModelError`, prediction with a feature provider. Source:
  https://developer.apple.com/documentation/coreml/mlmodel
  (REA evidence `ev_5c9b457f2dc27947b03852add0ed697cadeca81830845fb23834d2be05f6d0ed`).
  The loading methods (`init(contentsOf:)`, `load(contentsOf:configuration:)`) take a compiled model file, which
  BifrOSt does not read. The text form in `ml_model.rs` replaces that file and is documented as such.
- `MLState` and `MLStateConstraint` (state buffers for stateful models). Source:
  https://developer.apple.com/documentation/coreml/mlstate and
  https://developer.apple.com/documentation/coreml/mlstateconstraint
  (REA evidence `ev_682da7d5349a24f6bbd23339871c13a715a327f4e0343c027c9a0df989fa38d8`). The page says a state
  buffer must not be read or written while a prediction is in flight, and that stateful predictions on one state
  must be serialized. `ModelState` takes `&mut self` for every write, so Rust enforces that rule.

## Matrix rows closed

| Framework | Feature (as written in data/core-ml.json) | New status | BifrOSt path |
|---|---|---|---|
| core-ml | Loading and running a model | partial | springboard/ui/src/ml_model.rs |
| core-ml | Stateful models | partial | springboard/ui/src/ml_state.rs |

## Not done

- No compiled model files, no compute device choice, no batch prediction through the runner.
- Model encryption, runtime download and compile, custom layers and on-device updates stay gaps.
- Domain frameworks built on inference (vision, speech, language) stay gaps.
- Nothing was built or run in this wave, per the wave rules. The tests are written, not executed.
