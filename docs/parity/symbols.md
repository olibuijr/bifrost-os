# Symbols: symbol effects in pure Rust

Wave 2 of the parity matrix (`docs/parity-matrix/data/symbols.json`). The gaps closed here are the symbol effect
types, their options and the replace transition. They are modelled as pure functions of time, so each effect's pose
can be tested without a renderer.

## What was built

- `springboard/ui/src/symbol_effect.rs` (`bifrost_ui::symbol_effect`), a pure model:
  - `Effect`: appear, disappear, bounce, pulse, scale down, scale up, breathe, rotate, wiggle.
  - `Options` with `Repeat` (`NonRepeating`, `Count(n)`, `Indefinite`) and a speed multiplier. Zero, negative and NaN
    speeds fall back to 1.
  - `SymbolEffect { effect, options, start }` with `pose(now)`, `is_done(now)`, `cycles()` and `is_discrete()`. A
    discrete effect plays its cycles and rests at its end pose (a disappear ends hidden). An indefinite effect never
    finishes.
  - `Pose { scale, rotation, opacity, dx, dy }` with `REST` and `HIDDEN`.
  - `Replace` (`DownUp`, `OffUp`, `UpUp`) and `replace_poses(variant, progress)`. Only one symbol is drawn at any
    progress value; the handover is at 0.5.
  - Curves come from `bifrost_ui::anim::Curve`. Cycle lengths and amounts are our own values.
- `springboard/sdk/src/glyphs.rs`: `posed_bounds`, `posed_color` and `glyph_effect(s, ui, name, bounds, colour,
  &SymbolEffect, now)`. The pose scales and offsets the glyph bounds around the centre and multiplies the colour's
  alpha. Rotation is not drawn: glyphs have no rotation transform, so rotate and wiggle draw upright in the glyph path.
  No shell or app call site uses `glyph_effect` yet.

## Reference API names mirrored

Symbols framework (Mac Catalyst 17.0, macOS 14.0): `SymbolEffect`, `DiscreteSymbolEffect`,
`IndefiniteSymbolEffect`, `ContentTransitionSymbolEffect`, `TransitionSymbolEffect`, `SymbolEffectOptions`
(`repeating`, `nonRepeating`, `repeat(_:)`, `speed(_:)`), `AppearSymbolEffect`, `DisappearSymbolEffect`,
`BounceSymbolEffect`, `PulseSymbolEffect`, `ScaleSymbolEffect` (`up`, `down`), `BreatheSymbolEffect`,
`RotateSymbolEffect`, `WiggleSymbolEffect`, `ReplaceSymbolEffect` (`downUp`, `offUp`, `upUp`).

Sources: https://developer.apple.com/documentation/symbols, https://developer.apple.com/documentation/symbols/symboleffect,
https://developer.apple.com/documentation/symbols/discretesymboleffect,
https://developer.apple.com/documentation/symbols/indefinitesymboleffect,
https://developer.apple.com/documentation/symbols/symboleffectoptions,
https://developer.apple.com/documentation/symbols/replacesymboleffect,
https://developer.apple.com/documentation/symbols/scalesymboleffect.

## REA evidence

- `ev_358566d8f344c05a9b4a0c410614ac17f396a77e534f97529bc3381c0991b08b`: rendered page of
  https://developer.apple.com/documentation/symbols (the Symbols framework overview, with its effect and option topics).

## Matrix rows closed

| Framework slug | Feature (as written in data/symbols.json) | New status | BifrOSt path |
|---|---|---|---|
| symbols | Appear, disappear and scale effects | partial | springboard/ui/src/symbol_effect.rs; springboard/sdk/src/glyphs.rs (glyph_effect) |
| symbols | Bounce and pulse effects | partial | springboard/ui/src/symbol_effect.rs |
| symbols | Content transitions (replace, automatic) | partial | springboard/ui/src/symbol_effect.rs (replace_poses; automatic not modelled) |
| symbols | Discrete and indefinite effect protocols and options | partial | springboard/ui/src/symbol_effect.rs (SymbolEffect, Options, Repeat) |
| symbols | Breathe, rotate, wiggle, draw-on and draw-off effects | partial | springboard/ui/src/symbol_effect.rs (breathe, rotate, wiggle; no draw-on or draw-off; rotation not drawn by glyph_effect) |

Still open in this framework: variable color effect, symbol weights, scales and rendering modes, and the
SF Symbols-style glyph catalogue (`springboard/sdk/src/glyphs.rs`).

## Tests

`symbol_effect.rs` has 8 behaviour tests (options, discrete rest, indefinite bounds, counted repeat, appear and
disappear end states, speed scaling, rotate and wiggle, replace handover). `glyphs.rs` has 5 added tests (rest pose,
scale about the centre, offset, opacity alpha, pre-start pose). These were not built or run in this wave; the
instructions for this round forbid builds.
