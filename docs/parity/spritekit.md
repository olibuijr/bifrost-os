# Node actions and keyframes (parity, team A7)

The 2D game framework row of the parity matrix (`docs/parity-matrix/data/spritekit.json`) gains a pure model of
composable node actions and keyframe sequences in `springboard/ui/src/sk_action.rs`.

## What was built

- `Action` is a tree. `Wait(seconds)` takes time, `Report(id)` fires an id to the caller at that point, `Sequence` runs
  its children one after another, `Group` runs them together and finishes with the last child, `Repeat(body, n)` runs
  the body a counted number of times, and `RepeatForever(body)` runs it until removed. `Action::duration` gives the time
  an action takes, infinite for an endless repeat.
- `Runner` holds running actions. `run` returns a `Handle`, `advance(dt)` moves every action on by `dt` seconds and
  returns the report ids that fired, in order. `remove`, `remove_all`, `is_active` and `active_count` manage them. Negative
  and non-finite steps count as zero. An endless repeat whose body takes no time finishes after one pass.
- `Keyframes` maps a time to a value. Keys are `(time, value)` pairs in strictly increasing time order, and
  `Interpolation::Linear` or `Interpolation::Hold` fills the span between keys. Times outside the keys take the nearest
  key's value.

## Limits

- Nothing runs these actions yet. No view or node in the SDK holds a `Runner`, so the model is not visible to apps.
- Keys have no per-key easing or timing mode.
- Verified only by host unit tests in `sk_action.rs`; not yet tested on Midget.
