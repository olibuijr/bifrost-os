# Wave 3, theme-31: hosting third-party audio plug-ins

Theme: "Hosting third-party AU v2 and v3 plug-ins" (size 8, frameworks Audio Unit, Audio Toolbox, CoreAudioKit,
Core Audio, Virtualization). Branch `parity/w3-theme-31`, built on `parity/integration` (waves 1 and 2).

## What was built

Three modules, all pure Rust with std only and no hardware access. The render unit and component registry from
wave 2 (`springboard/ui/src/audio_render.rs`) are reused, not copied.

- `springboard/ui/src/plugin_host.rs` (`bifrost_ui::plugin_host`): the host model for a plug-in instance.
  - `ParameterSpec` and `Node` (groups and parameters) form a `ParameterTree`. Addresses must be unique. Values
    start at their defaults, writes are clamped to the range, read-only parameters refuse writes, and changes that
    move a value are queued for `take_changes`. `snapshot` and `restore` are atomic: a bad entry changes nothing.
  - `PresetStore` holds factory presets (numbers unique) and user presets. A user preset takes the next number.
    Factory presets cannot be deleted. `select` applies a preset atomically.
  - `Bus` is a bus with the channel counts it accepts. `Instance` ties a `RenderUnit`, the tree, the presets and
    the buses together. `Instance::set_format` checks that the bus is enabled and accepts the channel count before
    the render unit checks the rest. Bus enabling is refused while the unit is initialized.
  - `PluginVersion` (V2 or V3) records which plug-in format a component follows. Both versions use the same model.
- `springboard/ui/src/plugin_view.rs` (`bifrost_ui::plugin_view`): the view side.
  - `ViewConfiguration`: width, height and whether the host draws its own controls. Size must be finite and above 0.
  - `GenericView` builds one control per writable parameter in the tree's groups. Booleans become toggles, indexed
    values with names become choices (clamped to the last name), and everything else becomes a slider with a
    normalized position and a label in the unit. Read-only parameters get no control.
  - `PannerView`: a pan position from -1 to 1 with equal-power gains, a horizontal fraction input, and `apply` to
    write the position to the pan parameter.
- `springboard/sdk/src/system_sound.rs` (`bifrost_app::system_sound`): an app-facing system sound model.
  - `SystemSounds::create` takes an absolute path and returns a new id (ids start at 1 and are never reused).
  - `play` replaces any request still waiting, because only one sound plays at a time. `take_request` moves the
    waiting request to active for the audio service. `dispose` drops a waiting or active play. `finish` reports
    whether the app asked for a completion.

## Official API names mirrored

Each name below is from the official documentation, read through `tools/parity/docc.py` and the rendered page
(REA evidence listed under "Evidence").

- `AUAudioUnit`: `parameterTree`, `factoryPresets`, `currentPreset`, `fullState`, `inputBusses`/`outputBusses`
  (https://developer.apple.com/documentation/audiotoolbox/auaudiounit)
- `AUParameterTree`: `createTree(withChildren:)`, `createGroup(withIdentifier:name:children:)`,
  `createParameter(withIdentifier:name:address:min:max:unit:unitName:flags:valueStrings:dependentParameters:)`,
  `parameter(withAddress:)` (https://developer.apple.com/documentation/audiotoolbox/auparametertree)
- `AUAudioUnitPreset`: `name`, `number` (https://developer.apple.com/documentation/audiotoolbox/auaudiounitpreset)
- `AUAudioUnitBus`: `format`, `isEnabled`, `name`, `supportedChannelCounts`, `maximumChannelCount`
  (https://developer.apple.com/documentation/audiotoolbox/auaudiounitbus)
- `AUAudioUnitViewConfiguration`: `init(width:height:hostHasController:)`, `width`, `height`, `hostHasController`
  (https://developer.apple.com/documentation/coreaudiokit/auaudiounitviewconfiguration)
- `AUViewController`, `AUGenericView`, `AUPannerView`: `init(audioUnit:)`, `audioUnit`
  (https://developer.apple.com/documentation/coreaudiokit)
- `AudioServicesCreateSystemSoundID`, `AudioServicesDisposeSystemSoundID`, `AudioServicesPlaySystemSound`,
  `AudioServicesPlayAlertSound`, and their `...WithCompletion` forms (https://developer.apple.com/documentation/audiotoolbox/audio-services)

Where the official pages say nothing, the model chooses and says so in code: the pan law (equal power is our
choice), the user preset numbering rule, and the sound id start at 1.

## Matrix rows closed (status partial)

| Framework slug | Feature (as in docs/parity-matrix/data/<slug>.json) | Status | BifrOSt path |
|---|---|---|---|
| audio-unit | Hosting third-party AU v2 and v3 plug-ins | partial | springboard/ui/src/plugin_host.rs (Instance, PluginVersion, ParameterTree, PresetStore, Bus) |
| audio-toolbox | Audio Unit v3 plug-ins and AUAudioUnit host (parameters, presets, buses) | partial | springboard/ui/src/plugin_host.rs (ParameterTree, PresetStore, Bus, Instance) |
| coreaudiokit | Audio unit custom UI (view controller, view configuration, generic view) | partial | springboard/ui/src/plugin_view.rs (ViewConfiguration, GenericView, Control, Section) |
| coreaudiokit | Panner audio unit view | partial | springboard/ui/src/plugin_view.rs (PannerView) |
| audio-toolbox | System sounds and alert playback (Audio Services) | partial | springboard/sdk/src/system_sound.rs (SystemSounds, PlayRequest) |

Not closed by this theme, and left for later waves:

- audio-unit, "Implementing AU v3 plug-in extensions": there is no extension process or extension point. The model
  has the version tag only.
- core-audio, "Audio server driver plug-ins (virtual devices)": no user-space device plug-in loader exists, and
  loading one needs native code and device access, which this theme does not allow.
- virtualization, "Audio devices": the virtual machine route has no guest audio (tools/arch-vm.sh uses `-audio none`).

## Not done, stated plainly

- No plug-in binary is loaded. No component is registered by a real plug-in. The model accepts components only
  from `audio_render::ComponentRegistry`.
- No separate extension process, no MIDI, no musical context and no transport state.
- No view controller. The generic view and the panner are models only, with no drawing.
- Completions are flags on a request. The audio service has to call `finish` itself. No run loop exists.
- `audio_render` is still not wired into the apps or the shell (see the wave 2 note).
- The commands were not run, because this theme's rules say no builds and no tests. The tests in the three
  modules were written by hand and have not been executed. They need `cargo test -p bifrost-ui` and the sdk tests
  when a build is allowed.

## Evidence

- REA, AUAudioUnit page: `ev_496fbaf5e1f6e96928436f4c5b38318da261f1acb6651f14de8d2d6879fded3f`
- REA, CoreAudioKit landing page: `ev_d97c501f72811e7414175450ec0339d98cfedf9560255a255615ab79ed1f9c6a`
- REA, Audio Services landing page: `ev_2ab016a5b6700e667fd8df434354083fc4fd9f55ffb39ec6f7e883546b1d5c9e`

## Checks run

- `python3 tools/check-names.py`: no hits.
- No cargo, rustc, make or test was run (theme rule).
