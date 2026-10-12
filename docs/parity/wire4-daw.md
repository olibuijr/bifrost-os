# Wire 4: DAW (apps/daw) rack inspector, parameters and presets

Branch `wire4/daw`, app `springboard/apps/daw` (bifrost-daw), version 0.4.0. Built from release 0.5.40.
Scope: audio unit parameters and presets (the Rack view), and the stream format per track (core audio types,
channel layouts). Follows `docs/parity/wire2-daw.md`, which left the parameter tree unreachable.

## What is now wired

| Feature | Model | Wiring in the app |
|---|---|---|
| Parameter tree and sliders | `springboard/apps/daw/src/params.rs` (`tree_for`, `tree_from_extra`, `write_back`), `springboard/apps/daw/src/rack.rs` (`rows`, `value`, `set`, `position`, `value_at`), slider mapping from `springboard/ui/src/param_view.rs` (`ParamSpec`, linear or logarithmic) | `main.rs` `draw_rack`, `hit` (View::Rack), `rack_slide` |
| Instrument presets (save, apply, JSON) | `springboard/ui/src/plugin_host.rs` (`PresetStore`) through `params.rs` (`save_preset`, `apply_preset`, `presets_to_json`, `presets_from_json`), `rack.rs` (`add_preset`, `load_preset`, `load_presets`, `save_presets`, `preset_path`) | `main.rs` `rack_save_preset`, `rack_load_preset`, `refresh_rack_presets`; preset column in the Rack view |
| Stream format per track (rate, channels, layout, channel names) | `springboard/ui/src/audio_render.rs` (`StreamFormat`), `springboard/ui/src/channel_layout.rs` (`ChannelLayout`, `Tag`, `Label`) through `rack.rs` (`describe`) | `main.rs` `draw_rack` line "Stream: ..." under the Rack title |
| Previous and next track in the Rack | `main.rs` `step_rack_track` | header buttons "Previous Track" and "Next Track" in the Rack view |

The rack's slider values are stored in the song as plain numbers under each parameter's name, in the instrument's
or effect's `params`. A stored value outside the range is clamped on read and on write. Automation is unchanged: a
click on a parameter's name still opens it for automation in the Session view.

### How a user reaches each feature

- **Rack**: in a song's Session view, select a clip of the track, then tap "Rack". The Rack lists the instrument's
  parameters, then each effect's, one row each. Each row shows the name, a slider, and the value in the device's
  unit (Hz, dB, %, s, or a plain number). The track's name is in the title.
- **Set a value**: tap anywhere on a row's slider. The value jumps to that position, on the device's curve (a
  logarithmic curve for frequencies). The status line shows the new value, and the song is saved.
- **Stream format**: the line under the Rack title, "Stream: 48000 Hz · Stereo · Left, Right". Every track plays
  at the same render format, so the line reads the same for each track; it changes only if the render format changes.
- **Track**: "< Track" and "Track >" in the header step through the song's tracks.
- **Save Preset**: the button at the top of the right column saves the track's instrument values as "Preset 1",
  "Preset 2" and so on. The status line confirms the name. The presets are written to `<store>/presets/<kind>.json`
  (one file per instrument kind, shared by every song), so a preset saved in one song is listed in another.
- **Load Preset**: tap a preset's name in the right column. Its values are written to the instrument and the song is
  saved. The current preset is shown in the accent colour.

### Behaviour and approximations

- Presets are numbered automatically. Renaming and deleting presets are not wired, because the app has no
  text field for a name (see gaps).
- Presets are kept for the instrument only. The effects in the chain have no presets of their own.
- The stream format is the render format the DAW uses (48 kHz, two channels). The DAW never changes it per track,
  and the audio service carries only the rate and the channel count, so the per-track line is informational.
- A click on a slider sets the value; dragging is not followed (the app takes touch-down events only).
- The rack shows at most eight presets. The rows of a long chain are not scrolled: rows past the bottom of the
  window are not visible.

## Evidence (official documentation)

Reference behaviour comes from developer.apple.com, as the DocC data (`tools/parity/docc.py`) and as the rendered
page (`tools/parity/rea-page.sh`).

| Topic | Page | REA evidence id |
|---|---|---|
| AUPreset: factory presets of an audio unit (preset number and name), the preset model the rack follows | `audiotoolbox/aupreset` | `ev_4f4d55a650d2bf9380961990f8fac733a48bc2430c240e736b4366e5b5534901` |
| AudioStreamBasicDescription: sample rate, channels and format flags of a stream | `coreaudiotypes/audiostreambasicdescription` | `ev_f33c5ab4da8f5aa718f7fba79fe2bcbe7450c3a98cd97e508dac4d365d29a6b5` |
| AUParameterTree: the tree of groups and parameters (from wire 2) | `audiotoolbox/auparametertree` | `ev_00ee52df56320b213e450037b4253ad41fdcb2e021791d6779671055975c98ef` |

## How to verify on Midget

1. Publish and install the app on Midget (`ssh midget`, then `/usr/bin/springboard --install-store-app is.olibuijr.daw.desktop`).
2. Open DAW, open a song that has a track with an instrument (or make one with "+ Track"), and tap a clip of it.
3. Tap "Rack". Check the title, the "Stream: 48000 Hz · Stereo · Left, Right" line, and that each parameter has a
   slider with its value and unit. Tap a slider and check the value moves to that point and the status line shows it.
4. Tap "Save Preset". The status line reads "Saved Preset 1" and "Preset 1" appears in the right column.
   Change a slider, tap "Preset 1", and check the slider returns to the saved value.
5. Check `<store>/presets/<instrument>.json` exists and holds the values. Open another song and check its Rack lists
   "Preset 1" for the same instrument.
6. "Track >" and "< Track" move the title to the next and previous track. "Back" returns to the Session view.
7. Automation: tap a parameter's name (left of the slider) and check the Session view opens in automation mode for it.
8. Host tests: `cargo test -p bifrost-daw --test wire4_rack` (the rack model and preset helpers) and
   `cargo test -p bifrost-daw --test parity_wire` (the wire 2 parameter and preset tests still pass).

## Remaining gaps

- No preset rename or delete, and no name entry: presets are auto-named. The app has no text field in the Rack.
- Effects have no presets. The preset store is per instrument kind only.
- Sliders respond to a tap, not a drag. A drag needs the touch-move event, which the app does not take yet.
- The Rack does not scroll, so a long chain is cut off at the bottom of the window.
- No plug-in host: external Audio Unit and AUv3 plug-ins cannot load, so the parameter tree covers only the built-in
  devices. Audio unit "Parameters and presets" stays Partial.
- The stream format carries no bits per sample, bytes per frame or format flags, and no format ID. The audio service
  still passes only the rate and the channel count, so "AudioStreamBasicDescription and format flags" stays Partial.
- The channel layout model is used for the rack line only. Channel roles are not carried through the audio service
  or the speaker path, so "Channel layouts and channel descriptions" stays Partial.
- The generic view model (`param_view::GenericView`) is used only for its slider mapping. The rack does not show the
  expert parameter filter. The "Audio unit custom UI" row stays Partial.

## Matrix rows

| slug | feature (as in docs/parity-matrix/data/<slug>.json) | BifrOSt path |
|---|---|---|
| audio-unit | Parameters and presets | springboard/apps/daw/src/rack.rs, springboard/apps/daw/src/params.rs (`tree_from_extra`, `write_back`), springboard/apps/daw/src/main.rs (`draw_rack`, `rack_slide`, `rack_save_preset`, `rack_load_preset`) |
| core-audio-types | AudioStreamBasicDescription and format flags | springboard/apps/daw/src/rack.rs (`describe`), springboard/ui/src/audio_render.rs (`StreamFormat`) |
| core-audio-types | Channel layouts and channel descriptions | springboard/apps/daw/src/rack.rs (`describe`), springboard/ui/src/channel_layout.rs (`ChannelLayout`) |
| coreaudiokit | Audio unit custom UI (view controller, view configuration, generic view) | springboard/apps/daw/src/rack.rs (slider mapping over `springboard/ui/src/param_view.rs`) |
