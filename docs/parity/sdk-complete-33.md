# Parity sdk-complete-33: models added for the partial rows

Wave w8, task sdk-complete-33. The rows stay `partial`. This file lists what each new model mirrors, where it is wired and
what still needs the lab machine. Tests for all of them are in `springboard/<crate>/tests/parity_sdk-complete-33.rs`.

## Screen saver

| Module | Mirrors | Wired | Lab machine needed |
|---|---|---|---|
| `ui/src/screensaver_manifest.rs` (`SaverManifest`) | a module's declared name, frame interval, gamma fade, backing store and configuration options | SDK: `bifrost_app::screen_saver` | the shell does not load descriptors from a `.saver` bundle yet |
| `ui/src/screensaver.rs` (`ConfigureSheet::restore`) | applying saved preference values, skipping any the sheet refuses | SDK re-exports `ConfigureSheet` | Settings does not show the sheet |
| `ui/src/saver_random.rs` (`SaverRandom`) | a seeded source for the `SSRandom...` helpers (SplitMix64, no OS entropy) | SDK: `bifrost_app::screen_saver::SaverRandom` | the Clock saver does not use it yet |
| `render/src/gamma_fade.rs` (`FadeCycle`) | fade in after start, fade out after stop, full brightness when disabled | not wired to the display | the compositor has no gamma hook |
| `core/src/idle_monitor.rs` (`next_check_at`) | the time the next idle check is due | SDK: `IdleWatch::next_check_at` | the shell does not send its last-input time to apps |

## Metal-style views and textures

| Module | Mirrors | Wired | Lab machine needed |
|---|---|---|---|
| `ui/src/render_view.rs` (`RenderView`) | `MTKView` drawable size, automatic resizing, the loop and draw requests | not wired to a GPU view | a GPU view and a delegate need the presenter |
| `ui/src/texture_catalog.rs` (`TextureCatalog`) | image lookup by name with scale variants (1x, 2x, 3x) | not wired to a loader | the upload to GLES needs the GPU |

## Shaders and layout

| Module | Mirrors | Wired | Lab machine needed |
|---|---|---|---|
| `ui/src/effect_shader.rs` (`ShaderEffect`, `EffectKind`) | colour, distortion, layer and visual effect modifiers bound to a fragment function with checked uniforms | not wired to a pipeline | building a pipeline needs the GPU |
| `ui/src/lazy_stack.rs` | lazy list and grid windows (visible rows and items, overscan) | not wired into `Stack` | an SDK view that builds only visible rows |
| `render/src/filter_input.rs` | `CIFilter` input checks: known keys, finite values, ranges, defaults | not wired to a filter object | the filter implementations |

## Shared file list

| Module | Mirrors | Wired | Lab machine needed |
|---|---|---|---|
| `ui/src/shared_list_store.rs` (`encode`, `decode`) | keeping recents and favourites in a preference file | not yet used by a preference store | the Midget check of the recents menu |
| `ui/src/shared_list.rs` (`in_recency_order`, `from_recency_order`) | exact save and restore of list order | used by `shared_list_store` | none |

## Not done in this wave

- The replay kit microphone row needs the audio daemon and a device, so it has no change.
- The shader inventory waits for the macOS 27 extraction in `docs/macos-os-inventory.md`.
- The glkit lighting model is not modelled.
