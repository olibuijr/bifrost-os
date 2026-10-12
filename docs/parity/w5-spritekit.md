# Wave 5: SpriteKit scene lifecycle and positional audio (parity/w5-spritekit)

Pure Rust, std only, no unsafe, no app-facing SDK wiring yet. The new module is a model: the lead decides when apps
and the shell use it.

## What was built

`springboard/ui/src/scene2d.rs` (`bifrost_ui::scene2d`, one `pub mod` line in `springboard/ui/src/lib.rs`):

- `Scene2d`: a scene of fixed size shown in a view of another size. Four `ScaleMode`s (`Fill`, `AspectFill`,
  `AspectFit`, `ResizeFill`), an anchor fraction for placing the scaled scene in the view, the scene origin in view
  points, and conversion both ways between scene and view points. An empty scene axis keeps a factor of 1, and a zero
  view factor maps view points to 0, so no division by zero reaches the caller.
- `FramePhase` and `FRAME_PHASES`: the five frame-cycle phases in run order (update, evaluate actions, simulate
  physics, apply constraints, finish update).
- `FrameClock`: the update step for each frame from a caller-supplied time in seconds. The first frame has step 0, a
  clock that goes back gives step 0, and paused frames return `None` while still moving the reference time, so paused
  time is not replayed on resume.
- `positional_mix`: volume and pan (`Mix`) for an audio node heard from a listener node. Volume is 1 inside a reference
  distance, falls linearly to 0 at a max distance, and pan is the horizontal offset over the max distance. A
  non-positional node plays at full volume, centred.

The module does not draw, open audio output or read a clock. It has no camera node and no light falloff; the 3D
`scene_light.rs` already covers light attenuation for 3D scenes and is not duplicated.

## Official API names mirrored

Official pages read with `python3 -I tools/parity/docc.py` (DocC JSON) and checked against the rendered page:

- `SKScene`: `size`, `scaleMode`, `anchorPoint`, `update(_:)`, `didEvaluateActions()`, `didSimulatePhysics()`,
  `didApplyConstraints()`, `didFinishUpdate()`, `convertPoint(fromView:)`, `convertPoint(toView:)`
  (https://developer.apple.com/documentation/spritekit/skscene)
- `SKSceneScaleMode`: `fill`, `aspectFill`, `aspectFit`, `resizeFill`
  (https://developer.apple.com/documentation/spritekit/skscenescalemode)
- `SKAudioNode`: `isPositional`, positional mixing from the scene's `listener`
  (https://developer.apple.com/documentation/spritekit/skaudionode)

## REA evidence

- `ev_9be83454be472ad9b63487454441d5b45d38296db93613aa7ecffa332b60902f`: the SpriteKit landing page (recorded by wave 4).
- `ev_600c7bdce6c6b5e92875f39ed4f8bf6964db10f834f34f20ea93901950bd024e`: the `SKScene` page.
- `ev_b648b20aa6678a8e0835a717cb01a907fb8755d156bbb7cdcb9530607bf35f6a`: the `SKSceneScaleMode` page.
- `ev_9a5e96b5fdada4bade61fe114586d9b53658c023edaf8a02662eb99885ebc37b`: the `SKAudioNode` page.

## Matrix rows closed

Framework slug `spritekit`; the feature text is exactly as written in `docs/parity-matrix/data/spritekit.json`.

| Framework slug | Feature | New status | BifrOSt path |
|---|---|---|---|
| spritekit | Scene and scene lifecycle | partial | springboard/ui/src/scene2d.rs |
| spritekit | Environmental nodes (audio, light) | partial | springboard/ui/src/scene2d.rs (`positional_mix`); light falloff not covered here |

## Not done

- The `Custom shaders and uniforms` (os) and `Effect, crop and transform nodes` (os) rows need an app-facing SDK API
  over the GPU presenter and the glass pass, which is not a pure Rust change. They are left unchanged.
- `Renderer hand-off and custom Metal pipeline` (gap) needs the presenter to be exposed to apps. Not pure Rust.
- No builds or tests were run in this wave (the wave rules forbid it); the tests in `scene2d.rs` are written from
  the hand-worked values in the module and have not been executed.
