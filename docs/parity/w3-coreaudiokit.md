# Parity wave 3: CoreAudioKit (view layer for audio units)

Two pure models in `springboard/ui/src` (bifrost-ui, std only, no hardware access). Neither is wired into the shell or
an app yet; both are covered by host unit tests only, not yet run on the VM or Midget.

## What was built

- `springboard/ui/src/panner.rs`: a two-dimensional panner pad. Positions are kept in the unit square (x from -1 left
  to +1 right, y from -1 back to +1 front) and clamped. A window-local point maps through any rectangle, so the pad
  scales with its window. Stereo gains use the equal-power law (angle from hard left to a quarter turn), so the
  squared gains always sum to 1.
- `springboard/ui/src/param_view.rs`: the model behind a generic parameter view. Each parameter has a linear or
  logarithmic scale, an expert flag and a default. Expert parameters are hidden unless the display flags show them.
  Values clamp into range, and slider positions in 0 to 1 map to values and back. Parameters with an unusable range
  are dropped when the view is built.

Each module has a `#[cfg(test)] mod tests` block: 9 tests in `panner.rs`, 6 in `param_view.rs`. Expected values were
worked out by hand from the rules (for example, centre gains 1/sqrt(2), and the logarithmic position of 200 Hz on a
20 Hz to 20 kHz range is 1/3). Tests were not run: this wave does not build.

## Official API mirrored

- `AUPannerView` (CoreAudioKit, macOS 10.5): a view for a panner audio unit. Mirrored by `panner::PannerPosition`
  (position, pad mapping, stereo gains). The view and the audio unit host are not mirrored.
- `AUGenericView` and `AUGenericViewDisplayFlags` (CoreAudioKit, macOS 10.4): a generic view that shows every parameter
  of an audio unit. Mirrored by `param_view::GenericView`, `DisplayFlags::shows_expert` and `ParamSpec`.
- `AUAudioUnitViewConfiguration` and `AUViewController` are not mirrored. There is no audio unit host to supply them.

Official pages: https://developer.apple.com/documentation/coreaudiokit ,
https://developer.apple.com/documentation/coreaudiokit/aupannerview ,
https://developer.apple.com/documentation/coreaudiokit/augenericview

## REA evidence

| Page | evidence_id |
|---|---|
| CoreAudioKit (framework landing page) | `ev_7b49e6bca822a11c5f486754289256b12ded5e9aa8c332d1e128135224948296` |
| AUPannerView | `ev_1c42b3ae2914064e213c4ad7c20e23c28d309d77cd848ccd0d06bed9d4db0a19` |
| AUGenericView | `ev_5774013f79b078e2de98e6378ea8903620b44fbe89edeca622d3bf8d461b490d` |

## Matrix rows closed

Status is the new value proposed for the lead to apply centrally. The matrix data was not edited.

| Framework | Feature (as written in `docs/parity-matrix/data/coreaudiokit.json`) | New status | BifrOSt path |
|---|---|---|---|
| coreaudiokit | Panner audio unit view | partial | springboard/ui/src/panner.rs |
| coreaudiokit | Audio unit custom UI (view controller, view configuration, generic view) | partial | springboard/ui/src/param_view.rs |

The custom UI row stays partial: the generic view model is built, but there is no audio unit host, no view controller
and no view configuration.

## Not done

- Bluetooth MIDI pickers, the network audio device browser and the route picker are untouched. Each needs a device
  model and a discovery source that does not exist in pure Rust yet.
- The inter-device audio, inter-app audio and deprecated internals rows are marked n/a in the matrix and were left as
  they are.
