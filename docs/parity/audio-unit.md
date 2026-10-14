# Audio Unit parity (team J)

Branch `parity/team-j-audio-unit`, based on `arm`. Built on this branch and not yet verified on Midget.

## What was built

- `springboard/ui/src/audio_extension.rs` (new module, pure std): the extension point an app uses to ship an audio
  processor, following the shape of the AUAudioUnit subclassing contract.
  - `Manifest`: name, version, a four-character component description (kind, sub-kind, manufacturer), and the channel
    count of each input and output bus. Blank names, empty codes, a missing output bus, mismatched bus counts and
    channel counts outside 1 to 32 are refused.
  - `ExtensionRegistry`: keeps manifests in registration order, refuses a second manifest with the same description
    or name, and finds manifests by a query in which an empty code is a wildcard.
  - `ExtensionRegistry::allocate`: builds the render unit with the host's factory, sets the frame limit and every bus
    format at the host's sample rate, and initializes the unit. This is the allocation step before rendering.
  - `Session`: renders through the allocated unit and deallocates it. Rendering or a second deallocation after that
    returns `NotAllocated`.

The module reuses `audio_render` (`RenderUnit`, `StreamFormat`, `ComponentDescription`, `ComponentFactory`) and does
not duplicate its checks. No plug-in binary is loaded and no extension process is started. Applications cannot yet
ship a manifest; the shell has no loader that would call `register`.

## Relation to existing modules

- `plugin_host.rs` already models plug-in instances (parameter trees, presets, buses). The extension point is the
  layer below it that supplies a configured render unit.
- `audio_render.rs` provides the render unit and the component registry this module builds on.

## Tests

`#[cfg(test)] mod tests` in `audio_extension.rs` covers manifest validation, duplicate refusal, the wildcard query,
allocation of buses and rendering of the output channel count, the unknown index and bad sample rate refusals, a
factory with too few buses, the frame limit, and deallocation (once only, then no rendering). Tests were not run in
this round.

## Matrix rows

| Feature (`data/audio-unit.json`) | Status | BifrOSt path |
|---|---|---|
| Implementing AU v3 plug-in extensions | partial | springboard/ui/src/audio_extension.rs |

The row is partial, not parity: no loader, no Midget run, and no app ships a manifest yet.

## Not done

- A loader that reads manifests from installed apps and calls `ExtensionRegistry::register`.
- Hosting third-party binaries (the "Hosting third-party AU v2 and v3 plug-ins" row stays partial).
- Exposing sessions to other apps through the SDK.
