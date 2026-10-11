# Theme 08: render cycle, I/O start and stop, and component registry

Theme: "Audio Unit v2 render API (AudioUnit, render callbacks, I/O start and stop)" with its neighbours in
`docs/parity-matrix/themes.json`. This round closes the render-cycle core and the component lookup, in pure Rust.

## What was built

One new module, `springboard/ui/src/audio_render.rs` (registered by one `pub mod audio_render;` line in
`springboard/ui/src/lib.rs`). It is a pure model with no hardware access, no unsafe code and no new crates.

- `RenderUnit`: a processing node with input and output buses. It has per-bus stream formats (sample rate and
  channel count, planar 32-bit float), a maximum frames per slice, one render callback per input bus, and render
  notifications (pre and post, with an output-is-silence flag). `initialize` checks that bus 0 has an output
  format and that every callback bus has an input format at the output sample rate. Properties are locked while
  initialized. `render` runs the pre notifications, fills each callback block, mixes the blocks into the output,
  and runs the post notifications.
- `IoUnit`: wraps a `RenderUnit` and holds the start and stop state. `start` requires an initialized unit and
  resets the position. `pull` renders one block at the current sample time and advances it. `stop` and
  `uninitialize` behave as the state requires.
- `ComponentDescription` and `ComponentRegistry`: four-character kind, sub-kind and manufacturer codes, with an
  empty code as a wildcard. `find_next` mirrors the find-next walk over registered components, and `instantiate`
  creates a fresh `RenderUnit` from a factory.

Errors are a `RenderError` enum with a `Display` implementation. There are 9 behaviour tests in
`audio_render::tests`.

## Official API names mirrored

Taken from the official DocC index of the Audio Unit v2 (C) API and its topic groups (`audiotoolbox` via
`tools/parity/docc.py`):

- `AudioUnitInitialize`, `AudioUnitUninitialize`: `RenderUnit::initialize`, `RenderUnit::uninitialize`
- `AudioUnitRender`: `RenderUnit::render` (the pull of one block)
- `AURenderCallback` and `AURenderCallbackStruct`: `RenderCallback` and `RenderBlock`
- `AudioUnitAddRenderNotify`, `AudioUnitRemoveRenderNotify`: `RenderUnit::add_render_notify`,
  `RenderUnit::remove_render_notify`
- `AudioOutputUnitStart`, `AudioOutputUnitStop`: `IoUnit::start`, `IoUnit::stop`
- `AudioUnitSetProperty` and `AudioUnitGetProperty` for the stream format and the maximum frames per slice:
  `RenderUnit::set_stream_format`, `RenderUnit::stream_format`, `RenderUnit::set_max_frames`
- `AudioComponentFindNext`, `AudioComponentInstanceNew`, `AudioComponentDescription`: `ComponentRegistry::find_next`,
  `ComponentRegistry::instantiate`, `ComponentDescription`

Not mirrored: the property listener and parameter APIs, `AudioUnitGetParameter` and `AudioUnitSetParameter`, the
host callbacks, and the built-in unit classes.

## REA evidence

- `ev_0046e07da298c06e990ccf0f415b3e55697297d05971b9cc17a51f493a701ef5`: Audio Unit v2 (C) API landing page,
  `https://developer.apple.com/documentation/audiotoolbox/audio-unit-v2-c-api`. The rendered page lists the
  "Initializing the Audio Unit", "Starting and Stopping Output" and "Rendering the Audio" sections that this module
  follows.

## Matrix rows closed

| Framework slug | Feature (as in `docs/parity-matrix/data/<slug>.json`) | New status | BifrOSt path |
|---|---|---|---|
| audio-toolbox | Audio Unit v2 render API (AudioUnit, render callbacks, I/O start and stop) | partial | `springboard/ui/src/audio_render.rs` (`RenderUnit`, `IoUnit`, `RenderCallback`) |
| audio-toolbox | Audio components (registration, lookup, instantiation) | partial | `springboard/ui/src/audio_render.rs` (`ComponentRegistry`, `ComponentDescription`) |
| audio-unit | Audio rendering and offline render | partial | `springboard/ui/src/audio_render.rs` (`RenderUnit::render` without an I/O unit) |

## Still open, and why

- No SDK facade: the model is in `bifrost-ui` only. Apps cannot yet register a callback on the shell's audio
  service. That needs the service and a device thread, which this round does not add.
- No plug-in host for third-party processors (the AU v2 and v3 host rows, and the AUv3 parameter tree and
  presets). The factories are Rust functions; no external binary is loaded.
- Audio processing taps, sound event nodes and the audio engine graph remain gaps. They need more than a render
  callback.
- Voice I/O, spatial audio and the custom-UI rows are separate work.
