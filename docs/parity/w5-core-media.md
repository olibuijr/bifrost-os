# Core Media (parity wave 5)

Official reference: https://developer.apple.com/documentation/coremedia

REA evidence of the rendered pages:

- `CMTime` page (https://developer.apple.com/documentation/coremedia/cmtime):
  `ev_0d3f49fda05635b2a9fda5de953efa5114cd77e76e0284b50f41e24c97eba6b9`
- `CMTimebase` page (https://developer.apple.com/documentation/coremedia/cmtimebase):
  `ev_0574625193f6068a841ad05c73c50aeb7179ee2c3acf70c866c9b42428991645`

Official DocC facts were read with `tools/parity/docc.py` for `coremedia/cmtime`, `coremedia/cmtimebase`,
`coremedia/cmtimeroundingmethod` and `coremedia/cmtimerange`.

Waves 1 to 4 left the time type as internal microseconds and milliseconds, and the timebase as the player clock.
This round adds two pure models in `bifrost-ui` (`springboard/ui/src`) that mirror the app-facing time types. Neither is
wired into the media pipeline, the SDK or a shell call yet. The matrix rows move to `partial`.

## What was built

### `springboard/ui/src/media_time.rs` (`MediaTime`, `RoundingMethod`, `TimeRange`)

A rational time (value over timescale) with the non-numeric states, mirroring the time structure and its range type.

- Constructors: `new(value, timescale)` (a timescale that is not positive gives an invalid time), `with_epoch`,
  `from_seconds(seconds, preferred_timescale)` (NaN is invalid, infinities give the matching infinity, the rounding is
  recorded), `invalid`, `zero`, `indefinite`, `positive_infinity`, `negative_infinity`.
- Inspection: `value`, `timescale`, `epoch`, `is_valid`, `is_numeric`, `is_indefinite`, `is_positive_infinity`,
  `is_negative_infinity`, `has_been_rounded`, `seconds`.
- Arithmetic through `Add` and `Sub`. Two numeric times with different timescales are added on the larger timescale,
  with half-away-from-zero rounding of the operands that do not divide. Invalid operands give invalid, indefinite
  operands give indefinite, infinities absorb finite values, and positive plus negative infinity is invalid.
- `convert_scale(new_timescale, RoundingMethod)` with five rounding methods (half away from zero, away from zero, toward
  zero, toward positive and toward negative infinity). The default is half away from zero.
- `compare` returns `Option<Ordering>`, is exact across timescales with 128-bit products, and returns `None` for invalid
  and indefinite times.
- `TimeRange`: `new`, `from_start_end`, `invalid`, `zero`, `start`, `duration`, `end`, `is_valid`, `is_indefinite`,
  `is_empty`, `contains_time`.

Overflow never wraps: a result that does not fit in 64 bits becomes invalid.

Tests: 7 (validity, seconds rounding, addition rounding, non-numeric propagation, comparison, conversion rounding and
range containment). Expected values were worked out by hand from the rules above.

### `springboard/ui/src/media_timebase.rs` (`Timebase`, `TimebaseError`)

A timeline under application control. The reference clock time is passed in by the caller, so the model reads no clock.

- `new(reference)`: paused at zero, anchored to `reference`. A non-numeric reference is refused.
- `time_at(reference)`: anchor time plus the elapsed reference time times the rate. A paused timebase shows its anchor.
- `rate`, `set_rate(rate, reference)` (keeps the time reached so far, then changes the rate from that instant),
  `set_rate_and_anchor_time(rate, anchor_time, reference)`.
- `set_time(time, reference)`: keeps the rate and records a jump. `take_time_jumps()` drains the jumps, oldest first.
- One-shot timers: `add_timer(id, fire_at)` (same id replaces), `remove_timer(id)`, `timer_count()`,
  `due_timers(reference)` which returns and removes the timers at or before the time shown, in fire-time order.
- Non-finite rates and non-numeric anchors are refused with `TimebaseError`.

Tests: 7 (paused timebase, rate of two, rate change keeps the time reached, set time records one jump, timers fire once
in order, timers can be removed, refused inputs).

## Matrix rows closed

Rows of `docs/parity-matrix/data/core-media.json`. The new status is `partial`, since the models are not yet wired into
apps or the pipeline.

| Framework | Feature | New status | BifrOSt path |
|---|---|---|---|
| core-media | Time representation | partial | springboard/ui/src/media_time.rs (`MediaTime`, `TimeRange`) |
| core-media | Reference clocks and timebases | partial | springboard/ui/src/media_timebase.rs (`Timebase`) |

## Not done in this round

- Sample buffers, block buffers and buffer queues are unchanged.
- No effective rate across a chain of source timebases, no `CMTimeMapping` equivalent, and no `quickTime` rounding method.
- Timers are one-shot. There is no repeat, no host-time conversion and no notification to subscribers. Time jumps are
  kept as a list the caller drains.
- No SDK API for apps and no use from the media pipeline. The tests have not been run in this round, since the wave
  rules forbid builds here.
